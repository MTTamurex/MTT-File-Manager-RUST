use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegDeleteKeyW, RegGetValueW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

pub(super) fn read_string(
    key_path: &str,
    value_name: Option<&str>,
) -> Result<Option<String>, String> {
    let key_wide = wide(key_path);
    let value_name_wide = value_name.map(wide);
    let value_name_pointer = value_name_wide
        .as_ref()
        .map_or_else(PCWSTR::null, |value| PCWSTR(value.as_ptr()));
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            value_name_pointer,
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if missing(status.0) {
        return Ok(None);
    }
    if !status.is_ok() || value_type != REG_SZ {
        return Err(format!(
            "Could not read an MTT shell registry value (Windows error {}).",
            status.0
        ));
    }

    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            value_name_pointer,
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if !status.is_ok() {
        return Err(format!(
            "Could not read an MTT shell registry value (Windows error {}).",
            status.0
        ));
    }
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..length])))
}

pub(super) fn write_string(
    key_path: &str,
    value_name: Option<&str>,
    value: &str,
) -> Result<(), String> {
    let key_wide = wide(key_path);
    let value_name_wide = value_name.map(wide);
    let value_name_pointer = value_name_wide
        .as_ref()
        .map_or_else(PCWSTR::null, |value| PCWSTR(value.as_ptr()));
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let byte_count = u32::try_from(data.len() * std::mem::size_of::<u16>())
        .map_err(|_| "The MTT shell registry value is too large.".to_string())?;
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            value_name_pointer,
            REG_SZ.0,
            Some(data.as_ptr().cast()),
            byte_count,
        )
    };
    if status.is_ok() {
        Ok(())
    } else {
        Err(format!(
            "Could not write an MTT shell registry value (Windows error {}).",
            status.0
        ))
    }
}

pub(super) fn delete_value(key_path: &str, value_name: Option<&str>) -> Result<(), String> {
    let key_wide = wide(key_path);
    let value_name_wide = value_name.map(wide);
    let value_name_pointer = value_name_wide
        .as_ref()
        .map_or_else(PCWSTR::null, |value| PCWSTR(value.as_ptr()));
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            value_name_pointer,
        )
    };
    if status.is_ok() || missing(status.0) {
        Ok(())
    } else {
        Err(format!(
            "Could not remove an MTT shell registry value (Windows error {}).",
            status.0
        ))
    }
}

pub(super) fn delete_key_if_empty(key_path: &str) -> Result<(), String> {
    let key_wide = wide(key_path);
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
    };
    if missing(status.0) {
        return Ok(());
    }
    if !status.is_ok() {
        return Err(format!(
            "Could not inspect an MTT shell registry key (Windows error {}).",
            status.0
        ));
    }

    let mut subkey_count = 0u32;
    let mut value_count = 0u32;
    let status = unsafe {
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
    if !status.is_ok() {
        return Err(format!(
            "Could not inspect an MTT shell registry key (Windows error {}).",
            status.0
        ));
    }
    if subkey_count != 0 || value_count != 0 {
        return Ok(());
    }

    let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(key_wide.as_ptr())) };
    if status.is_ok() || missing(status.0) {
        Ok(())
    } else {
        Err(format!(
            "Could not remove an empty MTT shell registry key (Windows error {}).",
            status.0
        ))
    }
}

pub(super) fn key_exists(key_path: &str) -> Result<bool, String> {
    let key_wide = wide(key_path);
    let mut key = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
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
    } else if missing(status.0) {
        Ok(false)
    } else {
        Err(format!(
            "Could not inspect an MTT shell registry key (Windows error {}).",
            status.0
        ))
    }
}

pub(super) fn notify_shell_association_changed() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn missing(code: u32) -> bool {
    code == 2 || code == 3
}
