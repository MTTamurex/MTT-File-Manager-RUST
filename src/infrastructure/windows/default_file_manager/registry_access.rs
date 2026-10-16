use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegDeleteKeyW, RegGetValueW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RRF_RT_REG_SZ,
};

use super::registration::Registry;

pub(super) fn read_current_user_string(key_path: &str, value_name: &str) -> Option<String> {
    UserRegistry.read(key_path, value_name).ok().flatten()
}

pub(super) struct UserRegistry;

impl Registry for UserRegistry {
    fn delete_key_if_empty(&mut self, path: &str) -> Result<(), String> {
        let path = wide(path);
        let mut key = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                Some(0),
                KEY_READ,
                &mut key,
            )
        };
        if matches!(status.0, 2 | 3) {
            return Ok(());
        }
        status
            .ok()
            .map_err(|e| format!("Could not open shell registration: {e}"))?;
        let mut subkeys = 0;
        let mut values = 0;
        let status = unsafe {
            let status = RegQueryInfoKeyW(
                key,
                None,
                None,
                None,
                Some(&mut subkeys),
                None,
                None,
                Some(&mut values),
                None,
                None,
                None,
                None,
            );
            let _ = RegCloseKey(key);
            status
        };
        status
            .ok()
            .map_err(|e| format!("Could not inspect shell registration: {e}"))?;
        if subkeys == 0 && values == 0 {
            unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) }
                .ok()
                .map_err(|e| format!("Could not remove empty shell registration: {e}"))?;
        }
        Ok(())
    }

    fn read(&self, path: &str, name: &str) -> Result<Option<String>, String> {
        let path = wide(path);
        let name = wide(name);
        let mut size = 0;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if matches!(status.0, 2 | 3) {
            return Ok(None);
        }
        status
            .ok()
            .map_err(|e| format!("Could not read shell registration: {e}"))?;
        let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        }
        .ok()
        .map_err(|e| format!("Could not read shell registration: {e}"))?;
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Ok(Some(String::from_utf16_lossy(&buffer[..length])))
    }

    fn write(&mut self, path: &str, name: &str, value: &str) -> Result<(), String> {
        let path = wide(path);
        let name = wide(name);
        let value = wide(value);
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(name.as_ptr()),
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            )
        }
        .ok()
        .map_err(|e| format!("Could not write shell registration: {e}"))
    }

    fn delete(&mut self, path: &str, name: &str) -> Result<(), String> {
        let path = wide(path);
        let name = wide(name);
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                PCWSTR(name.as_ptr()),
            )
        };
        if matches!(status.0, 2 | 3) {
            return Ok(());
        }
        status
            .ok()
            .map_err(|e| format!("Could not remove shell registration: {e}"))
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
