use super::identity::verify_saved_region;
use super::resume::seed_existing_prefix;
use super::scheduler::ActiveChunk;
use super::target::{ExistingFile, TargetError, TargetMode, create_target};
use super::{Session, calculate_downloaded, uses_range_workers};
use crate::client::RemoteFileInfo;
use crate::engine::chunks::ChunkRange;
use crate::engine::model::{DownloadStatus, DuplicateChoice, DuplicatePrompt};
use crate::engine::worker::OVERLAP_BYTES;
use crate::storage::{Storage, StorageError};
use std::path::PathBuf;

/// Blocks sampled when an existing file is adopted, so requests do not grow with file size.
const ADOPT_SAMPLES: u64 = 32;

pub(super) struct PendingDuplicate {
    pub(super) prompt: DuplicatePrompt,
    request: DuplicateRequest,
}

enum DuplicateRequest {
    /// The added link matches the download that is already in the list.
    Link {
        url: String,
        save_path: PathBuf,
        num_chunks: usize,
    },
    /// The resolved target file already exists on disk.
    Target {
        save_path: PathBuf,
        num_chunks: usize,
        info: RemoteFileInfo,
        existing: ExistingFile,
    },
}

/// Marks every byte of a file that is already on disk as downloaded.
fn completed_chunks(total: u64) -> Vec<ActiveChunk> {
    if total == 0 {
        return Vec::new();
    }
    let mut chunk = ActiveChunk::new(ChunkRange {
        id: 0,
        start: 0,
        end: total - 1,
    });
    chunk.downloaded = total;
    chunk.is_done = true;
    vec![chunk]
}

/// Sessions answer duplicate prompts through these entry points.
impl Session {
    /// Detects a repeated link while its entry is still in the download list.
    pub(super) fn duplicate_link(&self, url: &str) -> bool {
        self.status != DownloadStatus::Idle
            && !self.current_url.is_empty()
            && self.current_url == url
    }

    /// Publishes the prompt for a repeated link and waits for the user's answer.
    pub(super) async fn ask_link_duplicate(
        &mut self,
        url: String,
        save_path: PathBuf,
        num_chunks: usize,
    ) {
        let prompt = DuplicatePrompt {
            session_id: self.session_id,
            url: url.clone(),
            filename: self.current_filename.clone(),
            existing_bytes: self.file_info.as_ref().and_then(|info| info.content_length),
            link_duplicate: true,
        };
        let pending = PendingDuplicate {
            prompt,
            request: DuplicateRequest::Link {
                url,
                save_path,
                num_chunks,
            },
        };
        self.settle_duplicate(pending).await;
    }

    /// Publishes the prompt for a target file that is already on disk.
    pub(super) async fn ask_existing_target(
        &mut self,
        save_path: PathBuf,
        num_chunks: usize,
        info: RemoteFileInfo,
        existing: ExistingFile,
    ) {
        let pending = PendingDuplicate {
            prompt: existing.prompt(self.session_id, self.current_url.clone()),
            request: DuplicateRequest::Target {
                save_path,
                num_chunks,
                info,
                existing,
            },
        };
        self.settle_duplicate(pending).await;
    }

    /// Applies the remembered decision, or publishes the prompt and waits.
    async fn settle_duplicate(&mut self, pending: PendingDuplicate) {
        if let Some(choice) = self.duplicate_preference {
            self.apply_duplicate(pending, Some(choice)).await;
        } else {
            self.duplicate = Some(pending);
            self.publish_prompt();
        }
    }

    /// Reports the current state, so the UI sees a prompt appear or clear.
    pub(super) fn publish_prompt(&self) {
        let info = self.file_info.as_ref();
        self.publish(
            info.and_then(|info| info.content_length),
            calculate_downloaded(&self.active_chunks),
            0,
            None,
            info.is_some_and(uses_range_workers),
        );
    }

    /// Applies the answer to the pending duplicate prompt.
    pub(super) async fn resolve_duplicate(
        &mut self,
        session_id: u64,
        choice: Option<DuplicateChoice>,
    ) {
        // An answer for a replaced download must not clear the current prompt.
        if self
            .duplicate
            .as_ref()
            .is_none_or(|pending| pending.prompt.session_id != session_id)
        {
            return;
        }
        let Some(pending) = self.duplicate.take() else {
            return;
        };
        self.apply_duplicate(pending, choice).await;
    }

    /// Executes the user's or remembered decision for a pending duplicate.
    async fn apply_duplicate(
        &mut self,
        pending: PendingDuplicate,
        choice: Option<DuplicateChoice>,
    ) {
        match (pending.request, choice) {
            (DuplicateRequest::Link { .. }, None) => self.publish_prompt(),
            (DuplicateRequest::Target { .. }, None) => {
                self.status = DownloadStatus::Idle;
                self.current_speed = 0;
                self.publish(None, 0, 0, None, false);
            }
            (
                DuplicateRequest::Link {
                    url,
                    save_path,
                    num_chunks,
                },
                Some(DuplicateChoice::Numbered),
            ) => {
                self.begin_start(url, save_path, num_chunks, TargetMode::Numbered)
                    .await;
            }
            (
                DuplicateRequest::Link {
                    url,
                    save_path,
                    num_chunks,
                },
                Some(DuplicateChoice::Overwrite),
            ) => {
                self.begin_start(url, save_path, num_chunks, TargetMode::Overwrite)
                    .await;
            }
            (DuplicateRequest::Link { .. }, Some(DuplicateChoice::UseExisting)) => {
                self.keep_or_resume_existing().await;
            }
            (
                DuplicateRequest::Target {
                    save_path,
                    num_chunks,
                    info,
                    ..
                },
                Some(DuplicateChoice::Numbered),
            ) => {
                self.start_decided(info, save_path, num_chunks, TargetMode::Numbered)
                    .await;
            }
            (
                DuplicateRequest::Target {
                    save_path,
                    num_chunks,
                    info,
                    ..
                },
                Some(DuplicateChoice::Overwrite),
            ) => {
                self.start_decided(info, save_path, num_chunks, TargetMode::Overwrite)
                    .await;
            }
            (
                DuplicateRequest::Target { info, existing, .. },
                Some(DuplicateChoice::UseExisting),
            ) => self.adopt_existing(info, existing).await,
        }
    }

    /// Keeps the existing duplicate entry, resuming it only when it stopped.
    async fn keep_or_resume_existing(&mut self) {
        let resumable = self.snapshot_tx.borrow().resumable;
        match self.status.clone() {
            DownloadStatus::Paused => self.resume_download().await,
            DownloadStatus::Failed(_) if resumable => self.resume_download().await,
            // Running and completed downloads stay exactly as they are.
            _ => self.publish_prompt(),
        }
    }
}

/// Creates the target file for a decision the user already made.
impl Session {
    /// Creates a user-chosen target and starts the download that was already resolved.
    async fn start_decided(
        &mut self,
        info: RemoteFileInfo,
        save_path: PathBuf,
        num_chunks: usize,
        mode: TargetMode,
    ) {
        let created = tokio::task::spawn_blocking({
            let save_path = save_path.clone();
            let info_filename = info.filename.clone();
            let total_size = info.content_length;
            move || create_target(&save_path, &info_filename, total_size, mode)
        })
        .await;

        match created {
            Ok(Ok((filename, path, storage))) => {
                self.launch_download(info, filename, path, storage, num_chunks);
            }
            // Numbered and Overwrite always find a usable path, so this only guards a race.
            Ok(Err(TargetError::Existing { filename, path, .. })) => {
                self.fail_target(filename, path, &already_exists());
            }
            Ok(Err(TargetError::Storage {
                filename,
                path,
                source,
            })) => self.fail_target(filename, path, &source),
            Err(error) => {
                self.fail_early(format!("Could not create download file: {error}"));
            }
        }
    }

    /// Option 3 for an existing file: accept it, resume it, or refuse without touching it.
    async fn adopt_existing(&mut self, info: RemoteFileInfo, existing: ExistingFile) {
        let resumable = uses_range_workers(&info);
        let num_chunks = self.current_num_chunks;
        match info.content_length {
            Some(total) if total == existing.bytes => {
                self.adopt_complete(info, existing, total).await;
            }
            Some(total) if resumable && existing.bytes < total => {
                self.resume_existing(info, existing, total, num_chunks)
                    .await;
            }
            Some(total) if existing.bytes > total => {
                self.fail_existing(&existing, "it is larger than the remote file");
            }
            Some(_) => {
                self.fail_existing(&existing, "the server does not support resuming this file");
            }
            None => self.fail_existing(&existing, "the remote file size is unknown"),
        }
    }

    /// Accepts an on-disk file that already holds the full remote file.
    async fn adopt_complete(&mut self, info: RemoteFileInfo, existing: ExistingFile, total: u64) {
        let Some(storage) = self.open_existing(&existing, total).await else {
            return;
        };

        // A full-size file is only complete once its bytes match the remote file.
        if total > 0
            && let Err(reason) = self.verify_existing_bytes(&info, &storage, total).await
        {
            self.fail_existing(&existing, &reason);
            return;
        }

        self.set_target(existing.filename, existing.path, true);
        self.active_chunks = completed_chunks(total);
        self.active_storage = Some(storage);
        self.file_info = Some(info);
        self.status = DownloadStatus::Completed;
        self.current_speed = 0;
        self.publish(Some(total), total, 0, None, total > 0);
    }
}

fn already_exists() -> StorageError {
    StorageError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "target file already exists",
    ))
}

/// Resuming a file the engine did not write itself.
impl Session {
    /// Continues an on-disk file from its saved prefix and downloads the rest.
    async fn resume_existing(
        &mut self,
        info: RemoteFileInfo,
        existing: ExistingFile,
        total: u64,
        num_chunks: usize,
    ) {
        let Some(storage) = self.open_existing(&existing, total).await else {
            return;
        };

        if let Err(reason) = self
            .verify_existing_bytes(&info, &storage, existing.bytes)
            .await
        {
            self.fail_existing(&existing, &reason);
            return;
        }

        let validator = info.resume_validator().map(str::to_owned);
        self.set_target(existing.filename, existing.path, true);
        self.active_chunks = seed_existing_prefix(existing.bytes, total, num_chunks);
        self.spawn_pending_chunks(total, validator.as_deref(), &storage);
        self.enter_downloading(info, storage);
    }

    /// Opens an on-disk file at its remote size without truncating it.
    /// Reports the failure and returns None when it cannot be opened.
    async fn open_existing(&mut self, existing: &ExistingFile, total: u64) -> Option<Storage> {
        let opened = tokio::task::spawn_blocking({
            let path = existing.path.clone();
            move || Storage::create_or_open(&path, Some(total), false)
        })
        .await;
        match opened {
            Ok(Ok(storage)) => Some(storage),
            Ok(Err(error)) => {
                self.fail_existing(existing, &error.to_string());
                None
            }
            Err(error) => {
                self.fail_existing(existing, &format!("it could not be opened: {error}"));
                None
            }
        }
    }

    /// Samples evenly spaced blocks of an existing file, including the first and last, against
    /// the remote file. Files of up to `ADOPT_SAMPLES` blocks are checked whole.
    /// This consistency heuristic covers data the engine did not write itself.
    pub(super) async fn verify_existing_bytes(
        &mut self,
        info: &RemoteFileInfo,
        storage: &Storage,
        bytes: u64,
    ) -> Result<(), String> {
        if bytes == 0 {
            return Ok(());
        }
        let total = info
            .content_length
            .ok_or_else(|| "the remote file size is unknown".to_string())?;
        // The client is cloned so no non-`Sync` session state crosses an await point.
        let client = self.client.clone();
        let url = self.current_url.clone();
        let validator = info.resume_validator().map(str::to_owned);
        let sample_len = bytes.min(OVERLAP_BYTES);
        let span = bytes - sample_len;
        let steps = ADOPT_SAMPLES - 1;
        let mut offsets: Vec<u64> = (0..ADOPT_SAMPLES)
            .map(|i| span / steps * i + span % steps * i / steps)
            .collect();
        offsets.dedup();
        for offset in offsets {
            verify_saved_region(
                &client,
                &url,
                validator.as_deref(),
                total,
                storage,
                offset,
                sample_len,
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Reports an existing file that the engine refuses to reuse or replace.
    fn fail_existing(&mut self, existing: &ExistingFile, reason: &str) {
        self.current_filename.clone_from(&existing.filename);
        self.current_path.clone_from(&existing.path);
        self.owns_target = false;
        self.fail_early(format!(
            "Cannot use existing file {}: {reason}. Choose overwrite or a numbered copy instead",
            existing.path.display()
        ));
    }
}
