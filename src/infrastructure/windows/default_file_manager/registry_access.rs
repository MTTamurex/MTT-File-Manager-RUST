use windows::core::PCWSTR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE,
    RRF_RT_REG_SZ,
};

pub(super) fn read_current_user_string(key_path: &str, value_name: &str) -> Option<String> {
    let key_wide: Vec<u16> = key_path.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return None;
        }

        let mut value_size = 0u32;
        let mut value_type = REG_VALUE_TYPE(0);
        let size_result = RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            None,
            Some(&mut value_size),
        );
        if size_result.is_err() || value_size == 0 {
            let _ = RegCloseKey(key);
            return None;
        }

        let mut buffer = vec![0u16; (value_size as usize).div_ceil(2)];
        let result = RegGetValueW(
            key,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut value_size),
        );
        let _ = RegCloseKey(key);

        if result.is_err() {
            return None;
        }
        let length = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..length]))
    }
}
