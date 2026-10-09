use serde_json::json;
use std::path::{Path, PathBuf};

pub const HOST_NAME: &str = "com.kosmos.downloader";
pub const DEFAULT_EXTENSION_ID: &str = "ghnbdddbpdglebhbgiaffnkeioomhfkn";

#[must_use]
pub fn default_allowed_origins(additional_ids: &[String]) -> Vec<String> {
    let mut origins = vec![format!("chrome-extension://{DEFAULT_EXTENSION_ID}/")];
    for id in additional_ids {
        let clean = id.trim().trim_matches('/');
        let origin = if clean.starts_with("chrome-extension://") {
            format!("{clean}/")
        } else {
            format!("chrome-extension://{clean}/")
        };
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    origins
}

#[must_use]
pub fn generate_manifest(host_exe: &Path, origins: &[String]) -> serde_json::Value {
    json!({
        "name": HOST_NAME,
        "description": "Kosmos Download Manager Native Messaging Host",
        "path": host_exe.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": origins
    })
}

pub fn write_manifest(
    manifest_path: &Path,
    host_exe: &Path,
    origins: &[String],
) -> std::io::Result<()> {
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = generate_manifest(host_exe, origins);
    let content = serde_json::to_string_pretty(&manifest)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(manifest_path, content)
}

#[cfg(windows)]
pub fn register_host(
    host_exe: &Path,
    manifest_dir: Option<&Path>,
    additional_ids: &[String],
) -> std::io::Result<PathBuf> {
    let app_dir = manifest_dir.map_or_else(crate::platform::data_directory, Path::to_path_buf);
    let manifest_path = app_dir.join(format!("{HOST_NAME}.json"));
    let origins = default_allowed_origins(additional_ids);

    write_manifest(&manifest_path, host_exe, &origins)?;

    let reg_targets = [
        format!(r"HKCU\Software\Google\Chrome\NativeMessagingHosts\{HOST_NAME}"),
        format!(r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts\{HOST_NAME}"),
        format!(r"HKCU\Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\{HOST_NAME}"),
        format!(r"HKCU\Software\Chromium\NativeMessagingHosts\{HOST_NAME}"),
    ];

    let manifest_str = manifest_path.to_string_lossy();
    for key in &reg_targets {
        let status = reg(&[
            "add",
            key,
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            manifest_str.as_ref(),
            "/f",
        ])?;
        if !status.success() {
            eprintln!("[kdm-host] Warning: reg add failed for {key}: {status}");
        }
    }

    Ok(manifest_path)
}

#[cfg(not(windows))]
pub fn register_host(
    host_exe: &Path,
    _manifest_dir: Option<&Path>,
    additional_ids: &[String],
) -> std::io::Result<PathBuf> {
    let origins = default_allowed_origins(additional_ids);
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "HOME directory not found")
    })?;

    let targets = [
        home.join(".config/google-chrome/NativeMessagingHosts"),
        home.join(".config/chromium/NativeMessagingHosts"),
        home.join(".config/microsoft-edge/NativeMessagingHosts"),
        home.join(".config/BraveSoftware/Brave-Browser/NativeMessagingHosts"),
    ];

    let mut first_path = None;
    for dir in &targets {
        let manifest_path = dir.join(format!("{HOST_NAME}.json"));
        let _ = write_manifest(&manifest_path, host_exe, &origins);
        if first_path.is_none() {
            first_path = Some(manifest_path);
        }
    }

    Ok(first_path.unwrap_or_else(|| PathBuf::from(format!("{HOST_NAME}.json"))))
}

#[cfg(windows)]
fn reg(arguments: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let root = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .ok_or_else(|| std::io::Error::other("SystemRoot is not set"))?;
    let reg_exe = PathBuf::from(root).join("System32").join("reg.exe");

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    Command::new(reg_exe)
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_manifest() {
        let exe = Path::new(r"C:\Program Files\Kosmos\kosmos-download-manager.exe");
        let origins = vec!["chrome-extension://abc/".to_string()];
        let val = generate_manifest(exe, &origins);

        assert_eq!(val["name"], HOST_NAME);
        assert_eq!(val["type"], "stdio");
        assert_eq!(val["path"], exe.to_string_lossy().as_ref());
        assert_eq!(val["allowed_origins"][0], "chrome-extension://abc/");
    }

    #[test]
    fn test_default_origins_trust_only_own_extension() {
        assert_eq!(
            default_allowed_origins(&[]),
            vec![format!("chrome-extension://{DEFAULT_EXTENSION_ID}/")]
        );
    }

    #[test]
    fn test_default_origins_merges_unique() {
        let extra = vec![
            "ghnbdddbpdglebhbgiaffnkeioomhfkn".to_string(), // duplicate
            "another_extension_id".to_string(),
        ];
        let origins = default_allowed_origins(&extra);
        assert!(origins.contains(&format!("chrome-extension://{DEFAULT_EXTENSION_ID}/")));
        assert!(origins.contains(&"chrome-extension://another_extension_id/".to_string()));
    }
}
