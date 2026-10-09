use crate::support::fixtures::wait_for_snapshot;
use crate::support::http::{is_head_request, start_local_server};
use kosmos_download_manager::client::{HttpClient, RemoteFileInfo};
use kosmos_download_manager::engine::{DownloadAction, DownloadEngine, DownloadStatus};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn metadata_uses_head_fields_or_range_probe_total() {
    let client = HttpClient::new();
    for (head_status, head_ranges, probe_status, length, accepts_ranges, source) in [
        ("200 OK", true, "206 Partial Content", 64, true, "head"),
        (
            "206 Partial Content",
            true,
            "206 Partial Content",
            64,
            true,
            "head",
        ),
        (
            "405 Method Not Allowed",
            false,
            "206 Partial Content",
            128,
            true,
            "probe",
        ),
        ("405 Method Not Allowed", false, "200 OK", 1, false, "probe"),
        ("200 OK", false, "206 Partial Content", 64, true, "head"),
    ] {
        let addr = start_local_server(move |mut socket, request| async move {
            let is_head = is_head_request(&request);
            let (status, length, source, date) = if is_head {
                (head_status, 64, "head", "Wed, 16 Sep 2026 12:00:00 GMT")
            } else {
                assert!(
                    request
                        .lines()
                        .any(|line| line.eq_ignore_ascii_case("range: bytes=0-0"))
                );
                (probe_status, 1, "probe", "Thu, 17 Sep 2026 12:00:00 GMT")
            };
            let ranges = if !is_head || head_ranges {
                "Accept-Ranges: bytes\r\n"
            } else {
                ""
            };
            let body = if is_head { "" } else { "x" };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {length}\r\n\
                 Content-Range: bytes 0-0/128\r\n{ranges}\
                 Content-Disposition: attachment; filename={source}.bin\r\n\
                 ETag: \"{source}\"\r\nLast-Modified: {date}\r\n\
                 Connection: close\r\n\r\n{body}"
            );
            let _ = socket.write_all(response.as_bytes()).await;
        })
        .await;
        let url = format!("http://{addr}/fallback.bin");
        let info = tokio::time::timeout(Duration::from_secs(3), client.fetch_info(&url))
            .await
            .expect("Metadata lookup timed out")
            .expect("Metadata lookup failed");
        assert_eq!(
            info,
            RemoteFileInfo {
                content_length: Some(length),
                accepts_ranges,
                filename: format!("{source}.bin"),
                etag: Some(format!("\"{source}\"")),
                last_modified: Some(
                    if source == "head" {
                        "Wed, 16 Sep 2026 12:00:00 GMT"
                    } else {
                        "Thu, 17 Sep 2026 12:00:00 GMT"
                    }
                    .into()
                ),
            },
            "HEAD {head_status}, range support {head_ranges}, probe {probe_status}"
        );
    }
}

#[tokio::test]
async fn range_probe_with_unknown_total_does_not_report_probe_body_length() {
    for content_range in ["bytes 0-0/*", ""] {
        let addr = start_local_server(move |mut socket, request| async move {
            let response = if is_head_request(&request) {
                "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
            } else {
                let range = if content_range.is_empty() {
                    String::new()
                } else {
                    format!("Content-Range: {content_range}\r\n")
                };
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: 1\r\n{range}\
                     Connection: close\r\n\r\nx"
                )
            };
            let _ = socket.write_all(response.as_bytes()).await;
        })
        .await;
        let info = HttpClient::new()
            .fetch_info(&format!("http://{addr}/unknown.bin"))
            .await
            .expect("Metadata lookup failed");
        assert!(info.accepts_ranges, "Content-Range: {content_range:?}");
        assert_eq!(
            info.content_length, None,
            "Content-Range: {content_range:?}"
        );
    }
}

#[tokio::test]
async fn test_server_ignores_range_returns_200() {
    // Mock server returns 200 OK when a range was requested
    let addr = start_local_server(|mut socket, request| async move {
        if is_head_request(&request) {
            let resp = "HTTP/1.1 200 OK\r\n\
                        Content-Length: 1000\r\n\
                        Accept-Ranges: bytes\r\n\
                        Connection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
        } else {
            // Return 200 OK instead of 206 Partial Content.
            let resp = "HTTP/1.1 200 OK\r\n\
                        Content-Length: 1000\r\n\
                        Connection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
            let payload = [0u8; 1000];
            let _ = socket.write_all(&payload).await;
        }
    })
    .await;

    let temp_dir = std::env::temp_dir();
    let save_path = temp_dir.join(format!("kosmos_range_200_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&save_path);

    let engine = DownloadEngine::new();
    let action_tx = engine.action_tx();
    let mut snapshot_rx = engine.snapshot_rx();

    action_tx
        .send(DownloadAction::Start {
            url: format!("http://{addr}/test.bin"),
            save_path: save_path.clone(),
            num_chunks: 2,
        })
        .await
        .unwrap();

    let terminal = wait_for_snapshot(
        &mut snapshot_rx,
        Duration::from_secs(2),
        "Engine should detect range violation and fail",
        |snap| {
            matches!(
                snap.status,
                DownloadStatus::Completed | DownloadStatus::Failed(_)
            )
        },
    )
    .await;
    match terminal.status {
        DownloadStatus::Failed(err) => {
            assert!(
                err.contains("206 Partial Content"),
                "Expected 206 requirement error, got: {err}"
            );
        }
        DownloadStatus::Completed => {
            panic!("Engine must not complete when server returns 200 OK for sub-range requests!");
        }
        status => panic!("Expected a terminal download state, got {status:?}"),
    }
    let _ = std::fs::remove_file(&save_path);
}

#[tokio::test]
async fn test_probe_retries_rate_limit_before_failing() {
    // The first probe attempt (HEAD + ranged GET) is rate limited; the retry succeeds.
    let served = Arc::new(AtomicUsize::new(0));
    let addr = start_local_server(move |mut socket, request| {
        let n = served.fetch_add(1, Ordering::SeqCst);
        async move {
            let resp = if n < 2 {
                "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else if is_head_request(&request) {
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata"
            };
            let _ = socket.write_all(resp.as_bytes()).await;
        }
    })
    .await;

    let save_path =
        std::env::temp_dir().join(format!("kosmos_429_test_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&save_path);

    let engine = DownloadEngine::new();
    let action_tx = engine.action_tx();
    let mut snapshot_rx = engine.snapshot_rx();

    action_tx
        .send(DownloadAction::Start {
            url: format!("http://{addr}/limited.bin"),
            save_path: save_path.clone(),
            num_chunks: 2,
        })
        .await
        .unwrap();

    let terminal = wait_for_snapshot(
        &mut snapshot_rx,
        Duration::from_secs(5),
        "429 on the first probe should be retried, not fail the download",
        |snap| {
            matches!(
                snap.status,
                DownloadStatus::Completed | DownloadStatus::Failed(_)
            )
        },
    )
    .await;
    assert_eq!(terminal.status, DownloadStatus::Completed);
    assert_eq!(std::fs::read(&save_path).unwrap(), b"data");
    let _ = std::fs::remove_file(&save_path);
}
#[tokio::test]
async fn test_probe_bad_http_status() {
    // Mock server returning 404
    let addr = start_local_server(|mut socket, _request| async move {
        let resp = "HTTP/1.1 404 Not Found\r\n\
                    Content-Length: 0\r\n\
                    Connection: close\r\n\r\n";
        let _ = socket.write_all(resp.as_bytes()).await;
    })
    .await;

    let temp_dir = std::env::temp_dir();
    let save_path = temp_dir.join(format!("kosmos_404_test_{}.bin", std::process::id()));
    let _ = std::fs::remove_file(&save_path);

    let engine = DownloadEngine::new();
    let action_tx = engine.action_tx();
    let mut snapshot_rx = engine.snapshot_rx();

    action_tx
        .send(DownloadAction::Start {
            url: format!("http://{addr}/not_found.bin"),
            save_path: save_path.clone(),
            num_chunks: 2,
        })
        .await
        .unwrap();

    let terminal = wait_for_snapshot(
        &mut snapshot_rx,
        Duration::from_secs(1),
        "Engine should report Failed for 404 Not Found",
        |snap| {
            matches!(
                snap.status,
                DownloadStatus::Completed | DownloadStatus::Failed(_)
            )
        },
    )
    .await;
    match terminal.status {
        DownloadStatus::Failed(err) => {
            assert!(
                err.contains("404"),
                "Expected 404 error message, got: {err}"
            );
        }
        DownloadStatus::Completed => panic!("Engine must not complete for 404 Not Found"),
        status => panic!("Expected a terminal download state, got {status:?}"),
    }
    let _ = std::fs::remove_file(&save_path);
}
