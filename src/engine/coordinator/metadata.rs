use super::resume::{SavedDownload, resume_chunks_are_valid};
use super::scheduler::{MAX_CHUNK_RETRIES, retry_delay, spawn_download_workers};
use super::target::{ExistingFile, TargetError, TargetMode, create_target};
use super::{CoordinatorError, Session, calculate_downloaded, uses_range_workers};
use crate::client::{HttpClient, RemoteFileInfo, is_strong_etag};
use crate::engine::model::DownloadStatus;
use crate::engine::worker::WorkerError;
use crate::storage::{Storage, StorageError};
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

pub(super) enum FetchInfoKind {
    Start {
        save_path: PathBuf,
        num_chunks: usize,
        target: TargetMode,
    },
    Resume,
    Restart,
    Complete,
}

pub(super) struct FetchInfoMsg {
    pub(super) session_id: u64,
    pub(super) kind: FetchInfoKind,
    pub(super) result: Result<RemoteFileInfo, WorkerError>,
}

pub(super) fn spawn_info_fetch(
    session_id: u64,
    kind: FetchInfoKind,
    url: String,
    client: HttpClient,
    mut cancel_rx: watch::Receiver<bool>,
    info_tx: mpsc::Sender<FetchInfoMsg>,
    saved: Option<SavedDownload>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let result = tokio::select! {
            _ = cancel_rx.changed() => return,
            result = async {
                let mut retries = 0;
                loop {
                    let attempt: Result<_, WorkerError> = async {
                        let info = if matches!(kind, FetchInfoKind::Complete)
                            && let Some(saved) = &saved
                            && (!uses_range_workers(&saved.info)
                                || saved.info.etag.as_deref().is_some_and(is_strong_etag))
                        {
                            saved.info.clone()
                        } else {
                            client.fetch_info(&url).await?
                        };
                        if let Some(saved) = &saved && uses_range_workers(&saved.info) {
                            saved.verify(&client, &url, &info).await?;
                        }
                        Ok(info)
                    }.await;
                    if retries < MAX_CHUNK_RETRIES
                        && attempt.as_ref().is_err_and(WorkerError::is_retryable)
                    {
                        tokio::time::sleep(retry_delay(retries)).await;
                        retries += 1;
                        continue;
                    }
                    break attempt;
                }
            } => result,
        };

        if *cancel_rx.borrow() {
            return;
        }

        tokio::select! {
            _ = cancel_rx.changed() => {}
            _ = info_tx.send(FetchInfoMsg { session_id, kind, result }) => {}
        }
    })
}

impl Session {
    pub(super) async fn handle_info(&mut self, msg: FetchInfoMsg) {
        let expected_status = if matches!(msg.kind, FetchInfoKind::Complete) {
            DownloadStatus::Downloading
        } else {
            DownloadStatus::Connecting
        };
        if msg.session_id != self.session_id || self.status != expected_status {
            return;
        }

        self.info_handle = None;
        match (msg.kind, msg.result) {
            (FetchInfoKind::Complete, Ok(_)) => {
                let flush = if let Some(storage) = &self.active_storage {
                    let storage = storage.clone();
                    tokio::task::spawn_blocking(move || storage.sync())
                        .await
                        .map_err(CoordinatorError::StorageTask)
                        .and_then(|result| result.map_err(CoordinatorError::Storage))
                } else {
                    Ok(())
                };
                self.status = match flush {
                    Ok(()) => DownloadStatus::Completed,
                    Err(err) => {
                        DownloadStatus::Failed(format!("Failed to flush completed download: {err}"))
                    }
                };
                self.current_speed = 0;
                let downloaded = calculate_downloaded(&self.active_chunks);
                let total = self
                    .file_info
                    .as_ref()
                    .and_then(|info| info.content_length)
                    .unwrap_or(downloaded);
                self.publish(
                    Some(total),
                    downloaded,
                    0,
                    None,
                    self.status == DownloadStatus::Completed,
                );
            }
            (
                FetchInfoKind::Start {
                    save_path,
                    num_chunks,
                    target,
                },
                Ok(info),
            ) => {
                let created = tokio::task::spawn_blocking({
                    let save_path = save_path.clone();
                    let info_filename = info.filename.clone();
                    let total_size = info.content_length;
                    move || create_target(&save_path, &info_filename, total_size, target)
                })
                .await;

                match created {
                    Ok(Ok((filename, path, storage))) => {
                        self.launch_download(info, filename, path, storage, num_chunks);
                    }
                    Ok(Err(TargetError::Existing {
                        filename,
                        path,
                        bytes,
                    })) => {
                        self.current_filename.clone_from(&filename);
                        self.current_path.clone_from(&path);
                        let existing = ExistingFile {
                            filename,
                            path,
                            bytes,
                        };
                        self.ask_existing_target(save_path, num_chunks, info, existing)
                            .await;
                    }
                    Ok(Err(TargetError::Storage {
                        filename,
                        path,
                        source,
                    })) => {
                        self.fail_target(filename, path, &source);
                    }
                    Err(error) => {
                        let error = CoordinatorError::StorageTask(error);
                        self.fail_early(format!(
                            "Could not create download file {}: {error}",
                            self.current_path.display()
                        ));
                    }
                }
            }
            (kind @ (FetchInfoKind::Resume | FetchInfoKind::Restart), Ok(info)) => {
                let Some(storage) = self.active_storage.as_ref() else {
                    self.fail_download("Missing storage for paused download".into(), false);
                    return;
                };

                let was_resumable =
                    !matches!(kind, FetchInfoKind::Restart) && self.range_download();
                let resume = match info.content_length {
                    Some(total_size)
                        if uses_range_workers(&info)
                            && self.file_info.as_ref().is_some_and(|previous| {
                                previous.resume_metadata_matches(&info)
                            })
                            && resume_chunks_are_valid(&self.active_chunks, total_size) =>
                    {
                        Some(total_size)
                    }
                    _ => None,
                };

                let total_size = info.content_length;
                if was_resumable {
                    if let Some(total_size) = resume {
                        self.status = DownloadStatus::Downloading;
                        for chunk in &mut self.active_chunks {
                            // Completed chunks acknowledge again so pausing final verification can resume.
                            chunk.is_done = false;
                            chunk.retries = 0;
                            self.worker_handles.push(chunk.spawn(
                                self.session_id,
                                &self.current_url,
                                &self.client,
                                total_size,
                                info.resume_validator(),
                                storage,
                                &self.cancel_tx,
                                &self.worker_tx,
                            ));
                        }
                        self.file_info = Some(info);

                        self.publish(
                            Some(total_size),
                            calculate_downloaded(&self.active_chunks),
                            0,
                            None,
                            true,
                        );
                    } else {
                        self.fail_download(
                            "Cannot safely resume: the remote file changed or could not be validated"
                                .into(),
                            false,
                        );
                    }
                } else {
                    let resize = tokio::task::spawn_blocking({
                        let storage = storage.clone();
                        move || storage.set_len(total_size.unwrap_or(0))
                    })
                    .await
                    .map_err(CoordinatorError::StorageTask)
                    .and_then(|result| result.map_err(CoordinatorError::Storage));
                    match resize {
                        Ok(()) => {
                            self.active_chunks.clear();
                            spawn_download_workers(
                                self.session_id,
                                &self.current_url,
                                &self.client,
                                &info,
                                self.current_num_chunks,
                                storage,
                                &self.cancel_tx,
                                &self.worker_tx,
                                &mut self.active_chunks,
                                &mut self.worker_handles,
                            );
                            self.file_info = Some(info);
                            self.status = DownloadStatus::Downloading;
                            self.publish(total_size, 0, 0, None, self.range_download());
                        }
                        Err(err) => {
                            self.fail_download(err.to_string(), false);
                        }
                    }
                }
            }
            (FetchInfoKind::Start { .. }, Err(err)) => {
                self.fail_download(err.to_string(), err.is_retryable());
            }
            (
                FetchInfoKind::Resume | FetchInfoKind::Restart | FetchInfoKind::Complete,
                Err(err),
            ) => {
                if err.is_content_changed() {
                    self.restart_required = true;
                    return;
                }
                self.fail_download(err.to_string(), err.is_retryable());
            }
        }
    }
}

impl Session {
    /// Writes a prepared target file and starts one worker per chunk.
    pub(super) fn launch_download(
        &mut self,
        info: RemoteFileInfo,
        filename: String,
        path: PathBuf,
        storage: Storage,
        num_chunks: usize,
    ) {
        let total_size = info.content_length;
        let resumable = uses_range_workers(&info);
        self.current_filename = filename;
        self.current_path = path;
        self.owns_target = true;
        spawn_download_workers(
            self.session_id,
            &self.current_url,
            &self.client,
            &info,
            num_chunks,
            &storage,
            &self.cancel_tx,
            &self.worker_tx,
            &mut self.active_chunks,
            &mut self.worker_handles,
        );
        self.active_storage = Some(storage);
        self.file_info = Some(info);
        self.status = DownloadStatus::Downloading;
        self.publish(total_size, 0, 0, None, resumable);
    }

    /// Reports a target file that could not be prepared.
    pub(super) fn fail_target(&mut self, filename: String, path: PathBuf, source: &StorageError) {
        self.current_filename = filename;
        self.current_path = path;
        self.owns_target = matches!(source, StorageError::CreatedFileInitialization(_));
        let message = match source {
            StorageError::Io(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                format!(
                    "Refusing to overwrite existing file {}. Choose a different path or remove it first: {source}",
                    self.current_path.display()
                )
            }
            _ => source.to_string(),
        };
        self.fail_early(message);
    }
}
