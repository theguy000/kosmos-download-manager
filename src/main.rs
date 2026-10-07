#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kosmos_download_manager::engine::DownloadEngine;
use kosmos_download_manager::host::{
    HostMessage, register_host, run_native_messaging_host, send_ipc_message,
};
use kosmos_download_manager::ui::run_app;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    kosmos_download_manager::platform::allow_foreground_activation();
    let args: Vec<String> = std::env::args().collect();

    // 1. Native Messaging Host mode
    // Chrome/Edge/Firefox launches the host with: [exe_path, "chrome-extension://..."]
    // Or explicit flag --native-messaging
    if args.iter().any(|a| {
        a == "--native-messaging"
            || a.starts_with("chrome-extension://")
            || a.starts_with("moz-extension://")
    }) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        return runtime
            .block_on(run_native_messaging_host())
            .map_err(Into::into);
    }

    // 2. Registration mode
    // e.g. kosmos-download-manager --register [--extension-id <id>]
    if args.iter().any(|a| a == "--register" || a == "-r") {
        let mut extra_ids = Vec::new();
        let mut iter = args.iter().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--extension-id" | "-e" => {
                    if let Some(id) = iter.next() {
                        extra_ids.push(id.clone());
                    }
                }
                _ => {}
            }
        }
        let exe = std::env::current_exe()?;
        let path = register_host(&exe, None, &extra_ids)?;
        eprintln!("Registered native messaging host at: {}", path.display());
        return Ok(());
    }

    // 3. Extract any download URL passed on CLI
    let mut initial_url: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--url" || arg == "-u" {
            if let Some(url) = iter.next() {
                initial_url = Some(url.clone());
            }
        } else if !arg.starts_with('-')
            && (arg.starts_with("http://")
                || arg.starts_with("https://")
                || arg.starts_with("ftp://"))
        {
            initial_url = Some(arg.clone());
        }
    }

    // 4. Multi-instance check: If an instance is already running, notify it and exit
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        if let Some(ref url) = initial_url {
            let msg = HostMessage::Download {
                url: url.clone(),
                filename: None,
                referrer: None,
                total_bytes: None,
                auto_start: false,
            };
            if runtime.block_on(send_ipc_message(&msg)).is_ok() {
                return Ok(());
            }
        } else if runtime
            .block_on(send_ipc_message(&HostMessage::Show))
            .is_ok()
        {
            return Ok(());
        }
    }

    // 5. Start primary GUI instance
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let _runtime_guard = runtime.enter();

    let engine = DownloadEngine::new();
    let action_tx = engine.action_tx();
    let snapshot_rx = engine.snapshot_rx();

    run_app(&action_tx, snapshot_rx, initial_url.as_deref())?;

    Ok(())
}
