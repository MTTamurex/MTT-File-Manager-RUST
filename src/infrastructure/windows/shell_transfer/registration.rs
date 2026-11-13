//! Read-only detection of "Register Copy Handler", NOT the preferred copy engine.
//! HandleCopy/ConfirmDrag belong to TeraCopy and must not gate Shell delegation.
//! Do not cache: registration can change while the file manager is running.

use windows::core::{GUID, PCWSTR};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CLASSES_ROOT, RRF_RT_REG_SZ};

#[cfg(test)]
#[path = "registration_tests.rs"]
mod tests;

const HANDLER_KEYS: [&str; 2] = [
    r"Directory\shellex\DragDropHandlers\TeraCopy",
    r"Folder\shellex\DragDropHandlers\TeraCopy",
];

fn read_default_value(subkey: &str) -> Option<String> {
    let key: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
    let mut bytes = 0u32;
    unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            PCWSTR(key.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut bytes),
        )
        .ok()
        .ok()?;
        // Bound allocation for malformed registrations; cover full-length
        // Windows paths without a fixed short DLL-path buffer.
        if bytes == 0 || bytes > 65_536 || !bytes.is_multiple_of(2) {
            return None;
        }
        let mut text = vec![0u16; bytes as usize / 2];
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            PCWSTR(key.as_ptr()),
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()
        .ok()?;
        if !bytes.is_multiple_of(2) {
            return None;
        }
        let text = text.get(..bytes as usize / 2)?;
        let end = text.iter().position(|&unit| unit == 0)?;
        String::from_utf16(&text[..end]).ok()
    }
}

fn class_server_key(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or(value);
    let clsid = GUID::try_from(value).ok()?;
    Some(format!(r"CLSID\{{{clsid:?}}}\InprocServer32"))
}

fn registered_with(mut read: impl FnMut(&str) -> Option<String>) -> bool {
    HANDLER_KEYS.iter().any(|key| {
        let Some(server_key) = read(key).and_then(|value| class_server_key(&value)) else {
            return false;
        };
        // A leftover handler key without a registered COM server is not a
        // usable integration. Do not instantiate or load the DLL to detect it.
        read(&server_key).is_some_and(|server| !server.trim().is_empty())
    })
}

pub(super) fn is_registered() -> bool {
    registered_with(read_default_value)
}
