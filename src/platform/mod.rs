use std::path::{Path, PathBuf};

pub(crate) const DATA_DIRECTORY_NAME: &str = "Kosmos Download Manager";

pub(crate) fn data_directory() -> PathBuf {
    if let Some(directory) = environment_path("LOCALAPPDATA") {
        return directory.join(DATA_DIRECTORY_NAME);
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(DATA_DIRECTORY_NAME)
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Opens `path` with the system handler on Windows. No-op on other platforms.
#[cfg(windows)]
pub(crate) fn open_file(path: &Path) {
    if !path.exists() {
        return;
    }

    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let wide_path: Vec<u16> = OsStr::new(path.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: ShellExecuteW is invoked with a valid, null-terminated UTF-16 path pointer.
    // Standard show flags (SW_SHOWNORMAL) are supplied, and execution is asynchronous.
    #[allow(unsafe_code)]
    unsafe {
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: isize,
                lpOperation: *const u16,
                lpFile: *const u16,
                lpParameters: *const u16,
                lpDirectory: *const u16,
                nShowCmd: i32,
            ) -> isize;
        }
        const SW_SHOWNORMAL: i32 = 1;
        let _ = ShellExecuteW(
            0,
            std::ptr::null(),
            wide_path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(not(windows))]
pub(crate) fn open_file(_path: &Path) {}

/// Brings the application window with the specified title to the foreground.
pub(crate) fn bring_window_to_front(title: &str) {
    #[cfg(windows)]
    {
        bring_window_to_front_windows(title);
    }
    #[cfg(not(windows))]
    {
        let _ = title;
    }
}

/// Allows background and child processes to set foreground window on Windows.
pub fn allow_foreground_activation() {
    #[cfg(windows)]
    {
        // SAFETY: AllowSetForegroundWindow is a standard Win32 user32 function.
        // ASFW_ANY (0xFFFF_FFFF) grants permission to all processes to take the foreground.
        #[allow(unsafe_code)]
        unsafe {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn AllowSetForegroundWindow(dwProcessId: u32) -> i32;
            }
            const ASFW_ANY: u32 = 0xFFFF_FFFF;
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn bring_window_to_front_windows(title: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    let wide_title: Vec<u16> = OsStr::new(title)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: Win32 API functions are called with valid null-terminated wide string pointer and checked handles.
    unsafe {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> isize;
            fn ShowWindow(hWnd: isize, nCmdShow: i32) -> i32;
            fn SetForegroundWindow(hWnd: isize) -> i32;
            fn BringWindowToTop(hWnd: isize) -> i32;
            fn GetForegroundWindow() -> isize;
            fn GetWindowThreadProcessId(hWnd: isize, lpdwProcessId: *mut u32) -> u32;
            fn AttachThreadInput(idAttach: u32, idAttachTo: u32, fAttach: i32) -> i32;
            fn SetWindowPos(
                hWnd: isize,
                hWndInsertAfter: isize,
                X: i32,
                Y: i32,
                cx: i32,
                cy: i32,
                uFlags: u32,
            ) -> i32;
            fn EnumWindows(
                lpEnumFunc: unsafe extern "system" fn(isize, isize) -> i32,
                lParam: isize,
            ) -> i32;
            fn IsWindowVisible(hWnd: isize) -> i32;
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetCurrentThreadId() -> u32;
            fn GetCurrentProcessId() -> u32;
        }

        let mut hwnd = FindWindowW(std::ptr::null(), wide_title.as_ptr());
        if hwnd == 0 {
            // Fallback: find any visible window belonging to the current process
            let current_pid = GetCurrentProcessId();
            struct EnumData {
                pid: u32,
                found_hwnd: isize,
            }
            let mut data = EnumData {
                pid: current_pid,
                found_hwnd: 0,
            };
            unsafe extern "system" fn enum_proc(w: isize, lparam: isize) -> i32 {
                // SAFETY: `lparam` was passed to EnumWindows as a valid pointer to a live `EnumData` stack allocation.
                let d = unsafe { &mut *(lparam as *mut EnumData) };
                let mut proc_id = 0u32;
                // SAFETY: GetWindowThreadProcessId is called with a checked pointer to a stack-allocated u32.
                unsafe { GetWindowThreadProcessId(w, std::ptr::addr_of_mut!(proc_id)) };
                // SAFETY: IsWindowVisible accepts any window handle and safely returns 0 if invalid.
                if proc_id == d.pid && unsafe { IsWindowVisible(w) } != 0 {
                    d.found_hwnd = w;
                    return 0;
                }
                1
            }
            EnumWindows(enum_proc, std::ptr::addr_of_mut!(data) as isize);
            hwnd = data.found_hwnd;
        }

        if hwnd != 0 {
            const SW_RESTORE: i32 = 9;
            const SW_SHOW: i32 = 5;
            const HWND_TOPMOST: isize = -1;
            const HWND_NOTOPMOST: isize = -2;
            const SWP_NOMOVE: u32 = 0x0002;
            const SWP_NOSIZE: u32 = 0x0001;
            const SWP_SHOWWINDOW: u32 = 0x0040;

            let fg_hwnd = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(fg_hwnd, std::ptr::null_mut());
            let cur_thread = GetCurrentThreadId();

            if fg_thread != 0 && fg_thread != cur_thread {
                AttachThreadInput(cur_thread, fg_thread, 1);
            }

            ShowWindow(hwnd, SW_SHOW);
            ShowWindow(hwnd, SW_RESTORE);
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);

            // Momentarily set topmost and then clear it to guarantee the window is placed above the browser
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );
            SetWindowPos(
                hwnd,
                HWND_NOTOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            );

            if fg_thread != 0 && fg_thread != cur_thread {
                AttachThreadInput(cur_thread, fg_thread, 0);
            }
        }
    }
}

pub(crate) fn default_download_directory() -> PathBuf {
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(userprofile).join("Downloads");
        if p.exists() {
            return p;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "Kosmos Download Manager";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Whether Kosmos Download Manager is registered to launch at user sign-in.
#[cfg(windows)]
pub(crate) fn startup_enabled() -> bool {
    startup_enabled_in(RUN_KEY, VALUE_NAME)
}

/// Adds or removes the current executable from the user's sign-in launch list.
#[cfg(windows)]
pub(crate) fn set_startup_enabled(enabled: bool) -> std::io::Result<()> {
    let executable = std::env::current_exe()?;
    let value = enabled.then(|| startup_value(&executable));
    set_startup_in(RUN_KEY, VALUE_NAME, value.as_deref())
}

#[cfg(not(windows))]
pub(crate) const fn startup_enabled() -> bool {
    false
}

#[cfg(not(windows))]
// Result keeps the signature identical to the Windows impl so shared callers stay cfg-free.
#[allow(clippy::unnecessary_wraps)]
pub(crate) fn set_startup_enabled(_enabled: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn startup_value(executable: &Path) -> String {
    // Windows splits the Run command line on spaces unless the path is quoted.
    format!("\"{}\"", executable.display())
}

// ponytail: presence is the whole state; parsing localized `reg query` output is fragile.
#[cfg(windows)]
fn startup_enabled_in(key: &str, value_name: &str) -> bool {
    reg(&["query", key, "/v", value_name]).is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn set_startup_in(key: &str, value_name: &str, value: Option<&str>) -> std::io::Result<()> {
    if value.is_none() && !startup_enabled_in(key, value_name) {
        return Ok(());
    }
    let status = match value {
        Some(value) => reg(&[
            "add", key, "/v", value_name, "/t", "REG_SZ", "/d", value, "/f",
        ])?,
        None => reg(&["delete", key, "/v", value_name, "/f"])?,
    };
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "reg.exe exited with {status}"
        )))
    }
}

#[cfg(windows)]
fn reg(arguments: &[&str]) -> std::io::Result<std::process::ExitStatus> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    Command::new(reg_executable()?)
        .args(arguments)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

#[cfg(windows)]
fn reg_executable() -> std::io::Result<PathBuf> {
    let root = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .ok_or_else(|| std::io::Error::other("SystemRoot is not set"))?;
    Ok(PathBuf::from(root).join("System32").join("reg.exe"))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn test_startup_value_quotes_paths_with_spaces() {
        assert_eq!(
            startup_value(Path::new(r"C:\Program Files\Kosmos\app.exe")),
            r#""C:\Program Files\Kosmos\app.exe""#
        );
    }

    #[test]
    fn environment_path_returns_none_for_unset_variables() {
        assert_eq!(environment_path("KOSMOS_HISTORY_TEST_UNSET"), None);
    }

    #[test]
    fn test_startup_registration_round_trip() {
        let key = format!(
            r"HKCU\Software\KosmosDownloadManager\tests\{}",
            std::process::id()
        );
        let value_name = "Kosmos Download Manager Test";
        let executable = std::env::current_exe().expect("test executable path");
        let value = startup_value(&executable);

        assert!(!startup_enabled_in(&key, value_name));

        set_startup_in(&key, value_name, Some(&value)).expect("register test startup value");
        assert!(startup_enabled_in(&key, value_name));

        set_startup_in(&key, value_name, None).expect("remove test startup value");
        assert!(!startup_enabled_in(&key, value_name));

        // Disabling an already-disabled value is a no-op, not an error.
        set_startup_in(&key, value_name, None).expect("repeat disable is idempotent");

        // Best-effort cleanup of the throwaway parent key; the value under test is already gone.
        let _ = reg(&["delete", r"HKCU\Software\KosmosDownloadManager", "/f"]);
    }
}
