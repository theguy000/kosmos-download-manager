use super::protocol::{HostMessage, HostResponse, MAX_MESSAGE_BYTES};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(windows)]
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\kdm_ipc";

#[cfg(not(windows))]
pub fn default_socket_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("kdm_ipc.sock")
}

pub async fn read_framed_json<R: AsyncRead + Unpin, T: serde::de::DeserializeOwned>(
    mut reader: R,
) -> std::io::Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = u32::from_ne_bytes(len_bytes) as usize;
    if len > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Framed message too large",
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    serde_json::from_slice(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub async fn write_framed_json<W: AsyncWrite + Unpin, T: serde::Serialize>(
    mut writer: W,
    val: &T,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(val)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let len_bytes = (bytes.len() as u32).to_ne_bytes();
    writer.write_all(&len_bytes).await?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(windows)]
pub async fn send_ipc_message(message: &HostMessage) -> std::io::Result<HostResponse> {
    send_ipc_message_to_pipe(DEFAULT_PIPE_NAME, message).await
}

#[cfg(windows)]
pub async fn send_ipc_message_to_pipe(
    pipe_name: &str,
    message: &HostMessage,
) -> std::io::Result<HostResponse> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let mut client = ClientOptions::new().open(pipe_name)?;
    write_framed_json(&mut client, message).await?;
    read_framed_json(&mut client).await
}

#[cfg(windows)]
pub async fn start_ipc_server<F>(handler: F) -> std::io::Result<()>
where
    F: Fn(HostMessage) -> HostResponse + Send + Sync + 'static,
{
    start_ipc_server_on_pipe(DEFAULT_PIPE_NAME, handler).await
}

#[cfg(windows)]
pub async fn start_ipc_server_on_pipe<F>(pipe_name: &str, handler: F) -> std::io::Result<()>
where
    F: Fn(HostMessage) -> HostResponse + Send + Sync + 'static,
{
    use tokio::net::windows::named_pipe::ServerOptions;

    let handler = Arc::new(handler);
    let mut server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(pipe_name)?;

    loop {
        server.connect().await?;
        let mut connected = server;
        // Pre-create the next instance so the next client is not rejected.
        server = match ServerOptions::new().create(pipe_name) {
            Ok(next) => next,
            Err(e) => {
                log_stderr(format!("Failed to create subsequent pipe instance: {e}"));
                break;
            }
        };

        let handler_clone = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Ok(inbound) =
                read_framed_json::<_, crate::host::protocol::InboundHostMessage>(&mut connected)
                    .await
            {
                let resp = handler_clone(inbound.into());
                let _ = write_framed_json(&mut connected, &resp).await;
            }
        });
    }

    Ok(())
}

#[cfg(not(windows))]
pub async fn send_ipc_message(message: &HostMessage) -> std::io::Result<HostResponse> {
    let path = default_socket_path();
    send_ipc_message_to_path(&path, message).await
}

#[cfg(not(windows))]
pub async fn send_ipc_message_to_path(
    path: &std::path::Path,
    message: &HostMessage,
) -> std::io::Result<HostResponse> {
    let mut stream = tokio::net::UnixStream::connect(path).await?;
    write_framed_json(&mut stream, message).await?;
    read_framed_json(&mut stream).await
}

#[cfg(not(windows))]
pub async fn start_ipc_server<F>(handler: F) -> std::io::Result<()>
where
    F: Fn(HostMessage) -> HostResponse + Send + Sync + 'static,
{
    let path = default_socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;
    let handler = Arc::new(handler);

    loop {
        let (mut stream, _) = listener.accept().await?;
        let handler_clone = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Ok(inbound) =
                read_framed_json::<_, crate::host::protocol::InboundHostMessage>(&mut stream).await
            {
                let resp = handler_clone(inbound.into());
                let _ = write_framed_json(&mut stream, &resp).await;
            }
        });
    }
}

fn log_stderr(msg: impl std::fmt::Display) {
    eprintln!("[kdm-ipc] {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_async_framed_round_trip() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let msg = HostMessage::Ping;

        let handle = tokio::spawn(async move {
            let received: HostMessage = read_framed_json(&mut server).await.unwrap();
            assert_eq!(received, HostMessage::Ping);
            let resp = HostResponse::ok_with_message("pong");
            write_framed_json(&mut server, &resp).await.unwrap();
        });

        write_framed_json(&mut client, &msg).await.unwrap();
        let resp: HostResponse = read_framed_json(&mut client).await.unwrap();
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.message.as_deref(), Some("pong"));

        handle.await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn test_windows_named_pipe_ipc() {
        let test_pipe_name = format!(r"\\.\pipe\kosmos_test_pipe_{}", std::process::id());
        let pipe_name_clone = test_pipe_name.clone();

        let server_task = tokio::spawn(async move {
            let _ = start_ipc_server_on_pipe(&pipe_name_clone, |msg| match msg {
                HostMessage::Ping => HostResponse::ok_with_message("pong"),
                HostMessage::Download { url, .. } => {
                    HostResponse::ok_with_message(format!("downloading:{url}"))
                }
                HostMessage::Show => HostResponse::ok(),
            })
            .await;
        });

        // Give the server a small moment to initialize the pipe
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ping_resp = send_ipc_message_to_pipe(&test_pipe_name, &HostMessage::Ping)
            .await
            .expect("send ping");
        assert_eq!(ping_resp.status, "ok");
        assert_eq!(ping_resp.message.as_deref(), Some("pong"));

        let dl_resp = send_ipc_message_to_pipe(
            &test_pipe_name,
            &HostMessage::Download {
                url: "https://example.com/test.zip".into(),
                filename: None,
                referrer: None,
                total_bytes: None,
                auto_start: false,
            },
        )
        .await
        .expect("send download");
        assert_eq!(dl_resp.status, "ok");
        assert_eq!(
            dl_resp.message.as_deref(),
            Some("downloading:https://example.com/test.zip")
        );

        server_task.abort();
    }
}
