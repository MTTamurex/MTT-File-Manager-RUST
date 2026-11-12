use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO,
};

/// Fail closed, default to No, and never enumerate or delete before confirmation.
pub(crate) fn confirm(hwnd: HWND, key: &str, count: usize) -> windows::core::Result<bool> {
    let text: Vec<u16> = rust_i18n::t!(key, count = count)
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let title: Vec<u16> = rust_i18n::t!("context_menu.delete")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    if result.0 == 0 {
        Err(windows::core::Error::from_thread())
    } else {
        Ok(result == IDYES)
    }
}
