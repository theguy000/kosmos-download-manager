use kosmos_download_manager::host::ipc::{read_framed_json, write_framed_json};
use kosmos_download_manager::host::protocol::{
    HostMessage, HostResponse, MAX_MESSAGE_BYTES, read_native_json, read_native_message,
    write_native_json, write_native_message,
};
use kosmos_download_manager::host::registry::{
    HOST_NAME, default_allowed_origins, generate_manifest,
};
use std::io::Cursor;
use std::path::Path;

#[test]
fn test_native_messaging_protocol_framing() {
    // 1. Valid round-trip of length-prefixed bytes
    let payload = b"{\"action\":\"ping\"}";
    let mut buffer = Vec::new();
    write_native_message(&mut buffer, payload).expect("write native message");

    assert_eq!(buffer.len(), 4 + payload.len());
    let mut cursor = Cursor::new(buffer);
    let decoded = read_native_message(&mut cursor)
        .expect("read native message")
        .expect("some message");
    assert_eq!(decoded, payload);

    // 2. Reject payloads exceeding MAX_MESSAGE_BYTES
    let oversized = (MAX_MESSAGE_BYTES as u32 + 1).to_ne_bytes();
    let mut cursor = Cursor::new(oversized);
    let err = read_native_message(&mut cursor).expect_err("oversized message must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    // 3. Handle EOF cleanly
    let mut empty_cursor = Cursor::new(Vec::new());
    assert_eq!(read_native_message(&mut empty_cursor).unwrap(), None);

    // 4. Handle truncated length header
    let mut truncated = Cursor::new(vec![1, 2]);
    assert!(read_native_message(&mut truncated).is_err());
}

#[test]
fn test_native_messaging_json_messages() {
    let msg = HostMessage::Download {
        url: "https://example.com/test_archive.zip".to_string(),
        filename: Some("test_archive.zip".to_string()),
        referrer: Some("https://example.com/".to_string()),
        total_bytes: Some(10_485_760),
        auto_start: false,
    };

    let mut buf = Vec::new();
    write_native_json(&mut buf, &msg).expect("write json");

    let mut cursor = Cursor::new(buf);
    let read_back: HostMessage = read_native_json(&mut cursor)
        .expect("read json")
        .expect("value exists");

    assert_eq!(read_back, msg);

    // Test action alias (e.g. uppercase "Download")
    let uppercase_action = br#"{"action":"Download","url":"https://example.com/test.zip"}"#;
    let mut cursor = Cursor::new(uppercase_action);
    let parsed: HostMessage = serde_json::from_reader(&mut cursor).expect("parse uppercase action");
    assert!(
        matches!(parsed, HostMessage::Download { ref url, .. } if url == "https://example.com/test.zip")
    );

    // Test untagged direct download message without "action" key
    let untagged_payload = br#"{"url":"https://example.com/direct.tar.gz"}"#;
    let mut cursor = Cursor::new(untagged_payload);
    let inbound: kosmos_download_manager::host::InboundHostMessage =
        serde_json::from_reader(&mut cursor).expect("parse untagged payload");
    let converted: HostMessage = inbound.into();
    assert!(
        matches!(converted, HostMessage::Download { ref url, .. } if url == "https://example.com/direct.tar.gz")
    );
}

#[test]
fn test_native_manifest_generation() {
    let host_path = Path::new(r"C:\Program Files\Kosmos\kosmos-download-manager.exe");
    let origins = default_allowed_origins(&["test_ext_id".to_string()]);
    let manifest = generate_manifest(host_path, &origins);

    assert_eq!(manifest["name"], HOST_NAME);
    assert_eq!(manifest["type"], "stdio");
    assert_eq!(manifest["path"], host_path.to_string_lossy().as_ref());
    assert!(manifest["allowed_origins"].is_array());
    let arr = manifest["allowed_origins"].as_array().unwrap();
    assert!(
        arr.iter()
            .any(|v| v == "chrome-extension://ghnbdddbpdglebhbgiaffnkeioomhfkn/")
    );
    assert!(arr.iter().any(|v| v == "chrome-extension://test_ext_id/"));
}

#[tokio::test]
async fn test_ipc_end_to_end_messaging() {
    let (mut client, mut server) = tokio::io::duplex(2048);

    // Spawn server worker that handles HostMessage
    let server_task = tokio::spawn(async move {
        let msg: HostMessage = read_framed_json(&mut server).await.unwrap();
        match msg {
            HostMessage::Download { url, .. } => {
                let resp = HostResponse::ok_with_message(format!("prompted:{url}"));
                write_framed_json(&mut server, &resp).await.unwrap();
            }
            HostMessage::Show => {
                let resp = HostResponse::ok_with_message("window_shown");
                write_framed_json(&mut server, &resp).await.unwrap();
            }
            HostMessage::Ping => {
                let resp = HostResponse::ok_with_message("pong");
                write_framed_json(&mut server, &resp).await.unwrap();
            }
        }
    });

    // Client sends Download request
    let download_msg = HostMessage::Download {
        url: "https://example.com/movie.mkv".to_string(),
        filename: Some("movie.mkv".to_string()),
        referrer: None,
        total_bytes: Some(1024),
        auto_start: false,
    };
    write_framed_json(&mut client, &download_msg).await.unwrap();

    let resp: HostResponse = read_framed_json(&mut client).await.unwrap();
    assert_eq!(resp.status, "ok");
    assert_eq!(
        resp.message.as_deref(),
        Some("prompted:https://example.com/movie.mkv")
    );

    server_task.await.unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn test_named_pipe_popup_trigger() {
    use kosmos_download_manager::host::ipc::{send_ipc_message_to_pipe, start_ipc_server_on_pipe};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let test_pipe = format!(r"\\.\pipe\kosmos_test_popup_{}", std::process::id());
    let pipe_for_server = test_pipe.clone();
    let popup_triggered = Arc::new(AtomicBool::new(false));
    let popup_flag = Arc::clone(&popup_triggered);

    let server_handle = tokio::spawn(async move {
        let _ = start_ipc_server_on_pipe(&pipe_for_server, move |msg| {
            if let HostMessage::Download { url, .. } = msg {
                if url == "https://example.com/file.iso" {
                    popup_flag.store(true, Ordering::SeqCst);
                }
                HostResponse::ok_with_message("download_prompted")
            } else {
                HostResponse::ok()
            }
        })
        .await;
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let send_res = send_ipc_message_to_pipe(
        &test_pipe,
        &HostMessage::Download {
            url: "https://example.com/file.iso".to_string(),
            filename: Some("file.iso".to_string()),
            referrer: None,
            total_bytes: None,
            auto_start: false,
        },
    )
    .await;

    assert!(send_res.is_ok());
    let resp = send_res.unwrap();
    assert_eq!(resp.status, "ok");
    assert_eq!(resp.message.as_deref(), Some("download_prompted"));
    assert!(
        popup_triggered.load(Ordering::SeqCst),
        "Download message must trigger the popup handler"
    );

    server_handle.abort();
}
