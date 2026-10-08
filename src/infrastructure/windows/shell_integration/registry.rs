use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegDeleteKeyW, RegGetValueW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, REG_VALUE_TYPE, RRF_RT_ANY,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

pub(super) const CONTEXT_MENU_ID: &str = "MTT.FileManager.OpenInMTT";
pub(super) const SHELL_INTEGRATION_STATE_KEY: &str = "Software\\MTT-File-Manager\\ShellIntegration";
pub(super) const CONTEXT_MENU_STATE_KEY: &str =
    "Software\\MTT-File-Manager\\ShellIntegration\\ContextMenu";
pub(super) const REGISTRY_CLASSES_ROOT: &str = "Software\\Classes";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct RegistryValueSnapshot {
    pub(super) value_type: u32,
    pub(super) bytes: Vec<u8>,
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn status_error(context: &str, status: WIN32_ERROR) -> String {
    format!("{context} (Windows error 0x{:08X})", status.0)
}

fn status_result(context: &str, status: WIN32_ERROR) -> Result<(), String> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(status_error(context, status))
    }
}

fn is_missing(status: WIN32_ERROR) -> bool {
    status.0 == ERROR_FILE_NOT_FOUND.0 || status.0 == ERROR_PATH_NOT_FOUND.0
}

fn value_name_ptr(name: Option<&[u16]>) -> PCWSTR {
    name.map_or_else(PCWSTR::null, |name| PCWSTR(name.as_ptr()))
}

pub(super) fn read_value(
    key_path: &str,
    value_name: Option<&str>,
) -> Result<Option<RegistryValueSnapshot>, String> {
    let key_path_wide = wide(key_path);
    let value_name_wide = value_name.map(wide);
    let value_name_ptr = value_name_ptr(value_name_wide.as_deref());
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path_wide.as_ptr()),
            value_name_ptr,
            RRF_RT_ANY,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if is_missing(status) {
        return Ok(None);
    }
    status_result("Could not read a Shell registry value", status)?;

    let mut bytes = vec![0; size as usize];
    if size > 0 {
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(key_path_wide.as_ptr()),
                value_name_ptr,
                RRF_RT_ANY,
                Some(&mut value_type),
                Some(bytes.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        status_result("Could not read a Shell registry value", status)?;
        bytes.truncate(size as usize);
    }

    Ok(Some(RegistryValueSnapshot {
        value_type: value_type.0,
        bytes,
    }))
}

pub(super) fn write_value(
    key_path: &str,
    value_name: Option<&str>,
    value: &RegistryValueSnapshot,
) -> Result<(), String> {
    let key_path_wide = wide(key_path);
    let value_name_wide = value_name.map(wide);
    let data = (!value.bytes.is_empty()).then_some(value.bytes.as_ptr().cast());
    let size =
        u32::try_from(value.bytes.len()).map_err(|_| "Registry value is too large".to_string())?;
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path_wide.as_ptr()),
            value_name_ptr(value_name_wide.as_deref()),
            value.value_type,
            data,
            size,
        )
    };
    status_result("Could not write a Shell registry value", status)
}

pub(super) fn write_string(
    key_path: &str,
    value_name: Option<&str>,
    value: &str,
) -> Result<(), String> {
    write_value(key_path, value_name, &registry_string(value))
}

pub(super) fn delete_value(key_path: &str, value_name: Option<&str>) -> Result<(), String> {
    let key_path_wide = wide(key_path);
    let value_name_wide = value_name.map(wide);
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path_wide.as_ptr()),
            value_name_ptr(value_name_wide.as_deref()),
        )
    };
    if is_missing(status) {
        Ok(())
    } else {
        status_result("Could not remove a Shell registry value", status)
    }
}

pub(super) fn key_exists(key_path: &str) -> Result<bool, String> {
    let key_path_wide = wide(key_path);
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
    };
    if status.is_ok() {
        unsafe {
            let _ = RegCloseKey(key);
        }
        Ok(true)
    } else if is_missing(status) {
        Ok(false)
    } else {
        Err(status_error(
            "Could not inspect a Shell registry key",
            status,
        ))
    }
}

pub(super) fn delete_key_if_empty(key_path: &str) -> Result<(), String> {
    let key_path_wide = wide(key_path);
    let mut key = HKEY::default();
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
    };
    if is_missing(open_status) {
        return Ok(());
    }
    status_result("Could not inspect a Shell registry key", open_status)?;

    let mut subkey_count = 0u32;
    let mut value_count = 0u32;
    let query_status = unsafe {
        RegQueryInfoKeyW(
            key,
            None,
            None,
            None,
            Some(&mut subkey_count),
            None,
            None,
            Some(&mut value_count),
            None,
            None,
            None,
            None,
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    status_result("Could not inspect a Shell registry key", query_status)?;
    if subkey_count != 0 || value_count != 0 {
        return Ok(());
    }

    let delete_status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(key_path_wide.as_ptr())) };
    if is_missing(delete_status) {
        Ok(())
    } else {
        status_result(
            "Could not remove an empty Shell registry key",
            delete_status,
        )
    }
}

pub(super) fn registry_string(value: &str) -> RegistryValueSnapshot {
    let bytes = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    RegistryValueSnapshot {
        value_type: REG_SZ.0,
        bytes,
    }
}

pub(super) fn registry_string_text(value: &RegistryValueSnapshot) -> Option<String> {
    if value.value_type != REG_SZ.0 {
        return None;
    }
    let mut units: Vec<u16> = value
        .bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    while units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16(&units).ok()
}

pub(super) fn current_executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("Could not locate the MTT executable: {error}"))
}

pub(super) fn command_for_path(executable: &Path) -> String {
    format!("\"{}\" \"%1\"", executable.to_string_lossy())
}

pub(super) fn notify_association_changed() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::{command_for_path, registry_string, registry_string_text};
    use std::path::Path;

    #[test]
    fn command_quotes_the_executable_and_shell_argument() {
        assert_eq!(
            command_for_path(Path::new(r"C:\Program Files\Example App\mtt.exe")),
            r#""C:\Program Files\Example App\mtt.exe" "%1""#
        );
    }

    #[test]
    fn registry_string_snapshot_round_trips_unicode() {
        let value = registry_string("Open in MTT 文件夹");
        assert_eq!(
            registry_string_text(&value).as_deref(),
            Some("Open in MTT 文件夹")
        );
    }
}
