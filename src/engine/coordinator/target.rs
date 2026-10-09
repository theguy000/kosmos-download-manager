//! Which file a download writes: naming, collision handling, and create/overwrite/ask.
//! Directory-vs-file resolution and numbered names live here, not in the duplicate prompt flow.

use crate::engine::model::DuplicatePrompt;
use crate::storage::{Storage, StorageError};
use std::path::{Path, PathBuf};

/// How the engine picks the target file of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TargetMode {
    /// Ask the user before touching a file that already exists.
    Ask,
    /// Append a numeric suffix to the server filename.
    Numbered,
    /// Reuse the target path, discarding existing contents.
    Overwrite,
}

/// Why the engine could not prepare a target file.
pub(super) enum TargetError {
    /// A file is already there and the user has to decide what happens to it.
    Existing {
        filename: String,
        path: PathBuf,
        bytes: u64,
    },
    Storage {
        filename: String,
        path: PathBuf,
        source: StorageError,
    },
}

/// The file on disk a duplicate prompt is about.
pub(super) struct ExistingFile {
    pub(super) filename: String,
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
}

impl ExistingFile {
    pub(super) fn prompt(&self, session_id: u64, url: String) -> DuplicatePrompt {
        DuplicatePrompt {
            session_id,
            url,
            filename: self.filename.clone(),
            existing_bytes: Some(self.bytes),
            link_duplicate: false,
        }
    }
}

/// Directory targets get the server filename; everything else is an explicit file path.
/// An extensionless path is a directory unless a file already occupies it.
fn is_directory_target(save_path: &Path) -> bool {
    let display = save_path.to_string_lossy();
    save_path.is_dir()
        || display.ends_with('/')
        || display.ends_with('\\')
        || (save_path.extension().is_none() && !save_path.is_file())
}

/// Creates the target file for `mode`, never replacing a file the user did not confirm.
pub(super) fn create_target(
    save_path: &Path,
    info_filename: &str,
    total_size: Option<u64>,
    mode: TargetMode,
) -> Result<(String, PathBuf, Storage), TargetError> {
    let (directory, filename) = if is_directory_target(save_path) {
        (save_path.to_path_buf(), info_filename.to_string())
    } else {
        let filename = save_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(info_filename)
            .to_string();
        (
            save_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            filename,
        )
    };

    match mode {
        TargetMode::Ask => {
            let path = directory.join(&filename);
            match Storage::create_new(&path, total_size) {
                Ok(storage) => Ok((filename, path, storage)),
                Err(StorageError::Io(error))
                    if error.kind() == std::io::ErrorKind::AlreadyExists =>
                {
                    match std::fs::metadata(&path) {
                        Ok(metadata) => Err(TargetError::Existing {
                            filename,
                            path,
                            bytes: metadata.len(),
                        }),
                        Err(source) => Err(TargetError::Storage {
                            filename,
                            path,
                            source: StorageError::Io(source),
                        }),
                    }
                }
                Err(source) => Err(TargetError::Storage {
                    filename,
                    path,
                    source,
                }),
            }
        }
        TargetMode::Numbered => match create_collision_free(&directory, &filename, total_size) {
            Ok(created) => Ok(created),
            Err((filename, path, source)) => Err(TargetError::Storage {
                filename,
                path,
                source,
            }),
        },
        TargetMode::Overwrite => {
            let path = directory.join(&filename);
            match Storage::create_or_open(&path, total_size, true) {
                Ok(storage) => Ok((filename, path, storage)),
                Err(source) => Err(TargetError::Storage {
                    filename,
                    path,
                    source,
                }),
            }
        }
    }
}

/// Upper bound on automatic `name_N` collision renames before failing.
/// ponytail: circuit breaker; a normal filesystem finds a free name on the first try.
const MAX_AUTO_RENAME_ATTEMPTS: u32 = 10_000;

pub(super) fn create_collision_free(
    save_path: &Path,
    info_filename: &str,
    total_size: Option<u64>,
) -> Result<(String, PathBuf, Storage), (String, PathBuf, StorageError)> {
    let mut candidate_name = info_filename.to_string();
    let mut candidate_path = save_path.join(&candidate_name);
    let mut index = 0;
    loop {
        if index > 0 {
            candidate_name = next_numbered_filename(info_filename, index);
            candidate_path = save_path.join(&candidate_name);
        }
        match Storage::create_new(&candidate_path, total_size) {
            Ok(storage) => return Ok((candidate_name, candidate_path, storage)),
            Err(StorageError::Io(err)) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                index += 1;
                if index >= MAX_AUTO_RENAME_ATTEMPTS {
                    let err = std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("no available filename after {MAX_AUTO_RENAME_ATTEMPTS} attempts"),
                    );
                    return Err((candidate_name, candidate_path, StorageError::Io(err)));
                }
            }
            Err(err) => return Err((candidate_name, candidate_path, err)),
        }
    }
}

pub(super) fn next_numbered_filename(original_name: &str, index: u32) -> String {
    let path = Path::new(original_name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(original_name);
    let ext = path.extension().and_then(|e| e.to_str());
    match ext {
        Some(ext) => format!("{stem}_{index}.{ext}"),
        None => format!("{stem}_{index}"),
    }
}
