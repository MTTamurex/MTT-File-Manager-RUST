use super::extract_shell_menu;
use crate::infrastructure::windows::com_scope::ComScope;
use std::path::PathBuf;
use windows::Win32::Storage::FileSystem::GetLogicalDrives;
use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

#[test]
#[ignore = "requires at least two installed drives and invokes installed Shell extensions"]
fn multiple_drives_extract_native_context_menu() {
    let com = ComScope::sta();
    assert!(com.is_initialized());
    let mask = unsafe { GetLogicalDrives() };
    let paths: Vec<_> = (0..26)
        .filter(|index| mask & (1 << index) != 0)
        .map(|index| PathBuf::from(format!("{}:\\", (b'A' + index) as char)))
        .collect();
    assert!(paths.len() >= 2, "requires at least two installed drives");
    let hwnd = unsafe { GetDesktopWindow() };
    for count in 2..=paths.len() {
        extract_shell_menu(hwnd, &paths[..count])
            .unwrap_or_else(|error| panic!("{count}-drive extraction failed: {error}"));
    }
}
