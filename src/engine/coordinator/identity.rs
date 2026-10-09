use crate::client::HttpClient;
use crate::engine::worker::{OVERLAP_BYTES, WorkerError};
use crate::storage::{Storage, StorageError};

/// Checks that `len` saved bytes at `start` still match the remote file.
///
/// Samples the first and last `OVERLAP_BYTES` of the region against the server. Sampling can
/// miss changes outside the checked regions, so it is a consistency heuristic, not a whole-file
/// proof. Every saved-bytes identity check (resume and use-existing-file) goes through here.
pub(super) async fn verify_saved_region(
    client: &HttpClient,
    url: &str,
    validator: Option<&str>,
    total: u64,
    storage: &Storage,
    start: u64,
    len: u64,
) -> Result<(), WorkerError> {
    let end = start
        .checked_add(len)
        .filter(|end| *end <= total)
        .ok_or(WorkerError::InvalidRange("Invalid saved byte region"))?;
    let sample_len = len.min(OVERLAP_BYTES);
    let mut offsets = vec![start];
    if len > sample_len {
        offsets.push(end - sample_len);
    }
    for offset in offsets {
        let storage = storage.clone();
        let expected = tokio::task::spawn_blocking(move || {
            let mut bytes = vec![0; sample_len as usize];
            storage.read_at(offset, &mut bytes)?;
            Ok::<_, StorageError>(bytes)
        })
        .await??;
        client
            .verify_range(url, offset, &expected, total, validator)
            .await?;
    }
    Ok(())
}
