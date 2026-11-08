//! Read-only detection of TeraCopy's opt-in. Missing/unreadable preferences
//! preserve the existing MTT engine. Do not cache: settings can change at runtime.

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

fn read_handle_copy() -> bool {
    let key = w!("Software\\Code Sector\\TeraCopy");
    let value = w!("HandleCopy");
    unsafe {
        // 4.0.5 uses REG_SZ. Also accept the conventional DWORD representation.
        let mut number = 0u32;
        let mut size = size_of::<u32>() as u32;
        if RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut number as *mut u32).cast()),
            Some(&mut size),
        )
        .is_ok()
        {
            return number == 1;
        }
        let mut text = [0u16; 8];
        size = size_of_val(&text) as u32;
        let result = RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        );
        result.is_ok() && enabled_text(&text, size as usize)
    }
}

fn enabled_text(text: &[u16], bytes: usize) -> bool {
    bytes == 2 * size_of::<u16>() && text.get(..2) == Some(&[b'1' as u16, 0])
}

fn handler_registered(subkey: PCWSTR) -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CLASSES_ROOT,
            subkey,
            Some(0),
            KEY_QUERY_VALUE,
            &mut key,
        )
        .is_err()
        {
            return false;
        }
        let _ = RegCloseKey(key);
        true
    }
}

pub(super) fn enabled() -> bool {
    read_handle_copy()
        && (handler_registered(w!("Directory\\shellex\\DragDropHandlers\\TeraCopy"))
            || handler_registered(w!("Folder\\shellex\\DragDropHandlers\\TeraCopy")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_enabled_preference_activates_teracopy() {
        assert!(enabled_text(&[b'1' as u16, 0], 4));
        assert!(!enabled_text(&[b'0' as u16, 0], 4));
        assert!(!enabled_text(&[], 0));
        assert!(!enabled_text(&[b'1' as u16], 2));
        assert!(!enabled_text(&[b'1' as u16, 0, b'2' as u16, 0], 8));
    }
}
