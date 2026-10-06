use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024; // 1 MB per Chromium specification

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HostMessage {
    #[serde(alias = "Download", alias = "DOWNLOAD")]
    Download {
        url: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        total_bytes: Option<u64>,
        #[serde(default)]
        auto_start: bool,
    },
    #[serde(alias = "Show", alias = "SHOW")]
    Show,
    #[serde(alias = "Ping", alias = "PING")]
    Ping,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum InboundHostMessage {
    Standard(HostMessage),
    DirectDownload {
        url: String,
        #[serde(default)]
        filename: Option<String>,
        #[serde(default)]
        referrer: Option<String>,
        #[serde(default)]
        total_bytes: Option<u64>,
        #[serde(default)]
        auto_start: bool,
    },
}

impl From<InboundHostMessage> for HostMessage {
    fn from(msg: InboundHostMessage) -> Self {
        match msg {
            InboundHostMessage::Standard(m) => m,
            InboundHostMessage::DirectDownload {
                url,
                filename,
                referrer,
                total_bytes,
                auto_start,
            } => HostMessage::Download {
                url,
                filename,
                referrer,
                total_bytes,
                auto_start,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl HostResponse {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            status: "ok".to_string(),
            message: None,
        }
    }

    #[must_use]
    pub fn ok_with_message(msg: impl Into<String>) -> Self {
        Self {
            status: "ok".to_string(),
            message: Some(msg.into()),
        }
    }

    #[must_use]
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            message: Some(msg.into()),
        }
    }
}

/// Reads a native messaging frame (4-byte native-endian length prefix followed by UTF-8 bytes).
/// Returns `Ok(None)` on clean EOF before any bytes are read.
pub fn read_native_message<R: Read>(mut reader: R) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_bytes = [0u8; 4];
    let mut read_bytes = 0;
    while read_bytes < 4 {
        match reader.read(&mut len_bytes[read_bytes..]) {
            Ok(0) => {
                if read_bytes == 0 {
                    return Ok(None);
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Unexpected EOF in length prefix",
                ));
            }
            Ok(n) => read_bytes += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }

    let length = u32::from_ne_bytes(len_bytes) as usize;
    if length > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Message length {length} exceeds {MAX_MESSAGE_BYTES} bytes limit"),
        ));
    }

    let mut buffer = vec![0u8; length];
    reader.read_exact(&mut buffer)?;
    Ok(Some(buffer))
}

/// Writes a native messaging frame (4-byte native-endian length prefix followed by payload).
pub fn write_native_message<W: Write>(mut writer: W, message: &[u8]) -> std::io::Result<()> {
    let length = message.len();
    if length > MAX_MESSAGE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Message length {length} exceeds {MAX_MESSAGE_BYTES} bytes limit"),
        ));
    }

    let len_bytes = (length as u32).to_ne_bytes();
    writer.write_all(&len_bytes)?;
    writer.write_all(message)?;
    writer.flush()?;
    Ok(())
}

pub fn read_native_json<R: Read, T: serde::de::DeserializeOwned>(
    reader: R,
) -> std::io::Result<Option<T>> {
    match read_native_message(reader)? {
        Some(bytes) => {
            let val = serde_json::from_slice(&bytes).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("JSON deserialization error: {e}"),
                )
            })?;
            Ok(Some(val))
        }
        None => Ok(None),
    }
}

pub fn write_native_json<W: Write, T: Serialize>(writer: W, val: &T) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(val).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("JSON serialization error: {e}"),
        )
    })?;
    write_native_message(writer, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_message_framing_round_trip() {
        let payload = b"{\"action\":\"ping\"}";
        let mut buffer = Vec::new();

        write_native_message(&mut buffer, payload).unwrap();
        assert_eq!(buffer.len(), 4 + payload.len());

        let mut cursor = Cursor::new(buffer);
        let read_back = read_native_message(&mut cursor).unwrap();
        assert_eq!(read_back, Some(payload.to_vec()));
    }

    #[test]
    fn test_empty_input_returns_none() {
        let cursor = Cursor::new(Vec::new());
        let res = read_native_message(cursor).unwrap();
        assert_eq!(res, None);
    }

    #[test]
    fn test_truncated_length_prefix_fails() {
        let cursor = Cursor::new(vec![1, 2]); // only 2 bytes instead of 4
        assert!(read_native_message(cursor).is_err());
    }

    #[test]
    fn test_oversized_message_is_rejected() {
        let oversized_len = (MAX_MESSAGE_BYTES as u32 + 10).to_ne_bytes();
        let cursor = Cursor::new(oversized_len);
        let err = read_native_message(cursor).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn test_json_round_trip() {
        let msg = HostMessage::Download {
            url: "https://example.com/file.zip".to_string(),
            filename: Some("file.zip".to_string()),
            referrer: Some("https://example.com/".to_string()),
            total_bytes: Some(1024),
            auto_start: false,
        };

        let mut buffer = Vec::new();
        write_native_json(&mut buffer, &msg).unwrap();

        let mut cursor = Cursor::new(buffer);
        let decoded: Option<HostMessage> = read_native_json(&mut cursor).unwrap();
        assert_eq!(decoded, Some(msg));
    }

    #[test]
    fn test_response_serialization() {
        let resp = HostResponse::ok_with_message("download_prompted");
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(json, r#"{"status":"ok","message":"download_prompted"}"#);

        let err_resp = HostResponse::error("failed to connect");
        let err_json = serde_json::to_string(&err_resp).unwrap();
        assert_eq!(
            err_json,
            r#"{"status":"error","message":"failed to connect"}"#
        );
    }
}
