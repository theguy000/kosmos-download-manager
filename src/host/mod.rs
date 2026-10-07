pub mod ipc;
pub mod protocol;
pub mod registry;

pub use ipc::{send_ipc_message, start_ipc_server};
pub use protocol::{
    HostMessage, HostResponse, InboundHostMessage, read_native_json, read_native_message,
    write_native_json, write_native_message,
};
pub use registry::{HOST_NAME, default_allowed_origins, register_host};

use std::path::PathBuf;

/// Locates the `kosmos-download-manager` main application executable.
pub fn find_app_executable() -> std::io::Result<PathBuf> {
    let current_exe = std::env::current_exe()?;
    let dir = current_exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "No parent directory found")
    })?;

    let candidate_names = if cfg!(windows) {
        ["kosmos-download-manager.exe", "kosmos-downloader.exe"]
    } else {
        ["kosmos-download-manager", "kosmos-downloader"]
    };

    for name in candidate_names {
        let p = dir.join(name);
        if p.exists() {
            return Ok(p);
        }
    }

    Ok(current_exe)
}

/// Dispatches a message received from the browser extension.
/// Either routes it to the running Kosmos Download Manager GUI via IPC,
/// or launches the application if it is not currently running.
pub async fn handle_host_message(message: HostMessage) -> HostResponse {
    crate::platform::allow_foreground_activation();

    // 1. Try sending to the currently running GUI instance
    match send_ipc_message(&message).await {
        Ok(response) => response,
        Err(_) => {
            // 2. Application is not running; launch it with the requested URL if applicable
            match message {
                HostMessage::Download { ref url, .. } => match launch_app_with_url(url) {
                    Ok(()) => HostResponse::ok_with_message("app_launched"),
                    Err(e) => HostResponse::error(format!("Failed to launch app: {e}")),
                },
                HostMessage::Show => match launch_app_blank() {
                    Ok(()) => HostResponse::ok_with_message("app_launched"),
                    Err(e) => HostResponse::error(format!("Failed to launch app: {e}")),
                },
                HostMessage::Ping => HostResponse::ok_with_message("pong_offline"),
            }
        }
    }
}

fn launch_app_with_url(url: &str) -> std::io::Result<()> {
    let exe = find_app_executable()?;
    std::process::Command::new(exe)
        .arg("--url")
        .arg(url)
        .spawn()?;
    Ok(())
}

fn launch_app_blank() -> std::io::Result<()> {
    let exe = find_app_executable()?;
    std::process::Command::new(exe).spawn()?;
    Ok(())
}

/// Runs the native messaging host loop reading from stdin and writing to stdout.
pub async fn run_native_messaging_host() -> std::io::Result<()> {
    // Ensure stdio is in binary mode
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();

    while let Some(inbound) =
        read_native_json::<_, crate::host::protocol::InboundHostMessage>(&mut stdin)?
    {
        let response = handle_host_message(inbound.into()).await;
        write_native_json(&mut stdout, &response)?;
    }

    Ok(())
}
