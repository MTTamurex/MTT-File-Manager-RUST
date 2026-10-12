use windows::core::{Error, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY,
};

use crate::E_FAIL_HR;

const MENU_KEY: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT";
const MENU_COMMAND_KEY: &str =
    "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT\\command";
const CONTEXT_MENU_STATE_KEY: &str = "Software\\MTT-File-Manager\\ShellIntegration\\ContextMenu";

fn win32_hresult(code: WIN32_ERROR) -> windows::core::HRESULT {
    if code.0 == 0 {
        windows::core::HRESULT(0)
    } else {
        windows::core::HRESULT(((code.0 & 0xffff) | 0x80070000) as i32)
    }
}

pub(super) fn read_string(
    key_path: &str,
    value_name: Option<&str>,
) -> windows::core::Result<Option<String>> {
    let key_wide: Vec<u16> = key_path.encode_utf16().chain(std::iter::once(0)).collect();
    let value_name_wide = value_name.map(|name| {
        name.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    });
    let value_name_pointer = value_name_wide
        .as_ref()
        .map_or_else(PCWSTR::null, |name| PCWSTR(name.as_ptr()));
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            value_name_pointer,
            RRF_RT_ANY,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if status.0 == ERROR_FILE_NOT_FOUND.0 || status.0 == ERROR_PATH_NOT_FOUND.0 {
        return Ok(None);
    }
    if !status.is_ok() {
        return Err(Error::from_hresult(win32_hresult(status)));
    }
    if value_type != REG_SZ {
        return Ok(None);
    }

    let mut bytes = vec![0; size as usize];
    if size > 0 {
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key_wide.as_ptr()),
                value_name_pointer,
                RRF_RT_ANY,
                Some(&mut value_type),
                Some(bytes.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if !status.is_ok() {
            return Err(Error::from_hresult(win32_hresult(status)));
        }
        bytes.truncate(size as usize);
    }

    let mut units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    while units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16(&units)
        .map(Some)
        .map_err(|_| Error::from_hresult(E_FAIL_HR))
}

pub(super) fn context_menu_is_enabled() -> bool {
    let Ok(Some(executable)) = read_string(CONTEXT_MENU_STATE_KEY, None) else {
        return false;
    };
    let Ok(Some(command)) = read_string(MENU_COMMAND_KEY, None) else {
        return false;
    };
    command.eq_ignore_ascii_case(&format!("\"{executable}\" --open-folder \"%1\""))
}

pub(super) fn context_menu_executable() -> windows::core::Result<String> {
    read_string(CONTEXT_MENU_STATE_KEY, None)?
        .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL_HR))
}

pub(super) fn menu_title() -> windows::core::Result<String> {
    Ok(read_string(MENU_KEY, None)?.unwrap_or_else(|| "Open in MTT".to_string()))
}

pub(super) fn menu_icon() -> windows::core::Result<Option<String>> {
    read_string(MENU_KEY, Some("Icon"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn context_command_requires_its_enabled_marker_and_matching_command() {
        let executable = r"C:\Program Files\Example\mtt-file-manager.exe";
        let command = format!("\"{executable}\" --open-folder \"%1\"");
        assert_eq!(
            command,
            r#""C:\Program Files\Example\mtt-file-manager.exe" --open-folder "%1""#
        );
    }
}
