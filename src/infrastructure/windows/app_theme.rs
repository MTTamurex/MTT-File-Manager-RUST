//! Reads the Windows "app mode" (Settings > Personalization > Colors >
//! "Choose your default app mode"), which is what the `System` theme follows.
//!
//! The value lives in `HKCU\...\Themes\Personalize\AppsUseLightTheme`
//! (DWORD: `1` = light apps, `0` = dark apps). It is deliberately not cached so
//! `System` can react to OS changes at runtime.
//!
//! This module also opts the process into dark-mode support
//! (`init_windows_dark_mode`) so that apps launched by the file manager
//! inherit the correct app mode on Windows 10.

use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::FARPROC;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD,
};

const PERSONALIZE_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
const APPS_USE_LIGHT_THEME: &str = "AppsUseLightTheme";
const UXTHEME_DLL: &[u8] = b"uxtheme.dll\0";

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

/// `uxtheme.dll` ordinal 135: the undocumented dark-mode opt-in entry point
/// (`SetPreferredAppMode` / `AllowDarkModeForApp`, depending on the OS build).
/// Introduced in Windows 10 1809; accepts `1` as "allow dark" on every build
/// where it exists.
const UXTHEME_PREFERRED_APP_MODE_ORDINAL: usize = 135;
const PREFERRED_APP_MODE_ALLOW_DARK: i32 = 1;

/// Opts this process into dark mode support. Must run before any window is
/// created.
///
/// The preferred app mode is inherited by child processes on Windows 10, so
/// without this opt-in the apps launched by the file manager resolve
/// `ShouldAppsUseDarkMode()` (winit/egui's system-theme probe) as light even
/// when the user's app mode is dark. No-op where the ordinal is unavailable
/// (Windows 10 versions before 1809).
pub fn init_windows_dark_mode() {
    unsafe {
        let Ok(module) = LoadLibraryA(PCSTR(UXTHEME_DLL.as_ptr())) else {
            return;
        };
        let entry = GetProcAddress(
            module,
            PCSTR(UXTHEME_PREFERRED_APP_MODE_ORDINAL as *const u8),
        );
        if entry.is_none() {
            return;
        }
        let set_preferred_app_mode: unsafe extern "system" fn(i32) =
            std::mem::transmute::<FARPROC, unsafe extern "system" fn(i32)>(entry);
        set_preferred_app_mode(PREFERRED_APP_MODE_ALLOW_DARK);
    }
}
