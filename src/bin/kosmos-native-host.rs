#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    kosmos_downloader::platform::allow_foreground_activation();
    let args: Vec<String> = std::env::args().collect();

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
        let path = kosmos_downloader::host::register_host(&exe, None, &extra_ids)?;
        eprintln!("Registered native messaging host at: {}", path.display());
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(kosmos_downloader::host::run_native_messaging_host())?;
    Ok(())
}
