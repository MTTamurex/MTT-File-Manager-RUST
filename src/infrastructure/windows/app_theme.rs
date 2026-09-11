//! Reads the Windows "app mode" (Settings > Personalization > Colors >
//! "Choose your default app mode"), which is what the `System` theme follows.
//!
//! The value lives in `HKCU\...\Themes\Personalize\AppsUseLightTheme`
//! (DWORD: `1` = light apps, `0` = dark apps). It is deliberately not cached so
//! `System` can react to OS changes at runtime.

use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD,
};

const PERSONALIZE_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const APPS_USE_LIGHT_THEME: &str = "AppsUseLightTheme";

/// Reads whether Windows is configured to use light mode for apps.
///
/// Returns `None` when the value is missing or unreadable (e.g. portable or
/// non-Windows environments).
pub fn read_windows_apps_use_light_theme() -> Option<bool> {
    let key_wide: Vec<u16> = PERSONALIZE_KEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let value_wide: Vec<u16> = APPS_USE_LIGHT_THEME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return None;
        }

        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let result = RegGetValueW(
            hkey,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);

        result.is_ok().then_some(value != 0)
    }
}

/// Whether the OS app mode is currently dark. Unknown values fall back to
/// light. This is the single source of truth for the `System` theme mode.
pub fn windows_app_is_dark() -> bool {
    read_windows_apps_use_light_theme() == Some(false)
}
