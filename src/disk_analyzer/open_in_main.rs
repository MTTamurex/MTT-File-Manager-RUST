//! Sends "open folder" requests from the analyzer process to the main app.
//!
//! Primary channel: WM_COPYDATA to the main window (validated by magic in
//! the window subclass). Fallback: spawn the executable in `--open-path`
//! helper mode, which persists the request to the shared app-state DB for
//! the main instance to consume.

/// Must match the main window title constant in `main.rs` (never changes).
const MAIN_WINDOW_TITLE: &str = "MTT File Manager";
/// Mirrors the receiver-side payload cap in `window_subclass.rs`.
const MAX_REQUEST_BYTES: usize = 8192;

/// Ask the main app to navigate to `path`. Never fails visibly: both
/// channels are best-effort with logging.
pub fn open_path_in_main_app(path: &str) {
    #[cfg(target_os = "windows")]
    if send_to_main_window(path) {
        log::info!("[DISK-ANALYZER] open-in-main request sent: {path}");
        return;
    }

    spawn_open_path_helper(path);
}

#[cfg(target_os = "windows")]
fn send_to_main_window(path: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindow, SendMessageW, WM_COPYDATA};

    let bytes = path.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_REQUEST_BYTES {
        return false;
    }

    unsafe {
        let title: Vec<u16> = MAIN_WINDOW_TITLE
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let Ok(hwnd) = FindWindowW(None, PCWSTR(title.as_ptr())) else {
            return false;
        };
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }

        let mut copy_data = COPYDATASTRUCT {
            dwData: crate::infrastructure::windows::window_subclass::OPEN_REQUEST_MAGIC,
            cbData: bytes.len() as u32,
            lpData: bytes.as_ptr() as *mut std::ffi::c_void,
        };
        // The subclass handler only copies the payload into a queue, so this
        // synchronous send returns promptly even under load.
        let delivered = SendMessageW(
            hwnd,
            WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM(&mut copy_data as *mut COPYDATASTRUCT as isize)),
        )
        .0
            == 1;
        if delivered {
            // Bring the main window to the front from HERE: this process is
            // the foreground one (the user just clicked it), which is the
            // only moment Windows reliably allows stealing foreground. By
            // the time the main app's update loop reacts, the permission
            // window has already closed.
            crate::infrastructure::windows::restore_window_foreground(hwnd);
        }
        delivered
    }
}

fn spawn_open_path_helper(path: &str) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            log::error!("[DISK-ANALYZER] open-in-main fallback failed (current_exe): {error}");
            return;
        }
    };
    match std::process::Command::new(exe)
        .arg("--open-path")
        .arg(path)
        .spawn()
    {
        Ok(_) => log::info!("[DISK-ANALYZER] open-in-main fallback spawned for: {path}"),
        Err(error) => {
            log::error!("[DISK-ANALYZER] open-in-main fallback spawn failed: {error}")
        }
    }
}
