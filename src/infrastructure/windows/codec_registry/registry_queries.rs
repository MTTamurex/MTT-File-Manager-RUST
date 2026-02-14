use windows::{
    core::{GUID, PCWSTR},
    Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_VALUE_TYPE, RRF_RT_REG_SZ,
    },
};

/// Query Windows Registry for CLSID friendly name
///
/// **Registry Path:**
/// `HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\{GUID}\`
///
/// **Value:** `FriendlyName` or `(Default)`
pub(super) fn query_registry_friendly_name(guid: &GUID) -> Option<String> {
    unsafe {
        // Format GUID as "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}"
        let guid_str = format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7]
        );

        // Try: HKLM\SOFTWARE\Classes\CLSID\{GUID}
        let key_path = format!("SOFTWARE\\Classes\\CLSID\\{}", guid_str);
        if let Some(name) = query_registry_string(&key_path, "FriendlyName") {
            return Some(name);
        }

        // Fallback: Try default value
        if let Some(name) = query_registry_string(&key_path, "") {
            if !name.is_empty() {
                return Some(name);
            }
        }

        // Try: HKLM\SOFTWARE\Classes\MediaFoundation\Transforms\{GUID}
        let mf_key_path = format!(
            "SOFTWARE\\Classes\\MediaFoundation\\Transforms\\{}",
            guid_str
        );
        if let Some(name) = query_registry_string(&mf_key_path, "FriendlyName") {
            return Some(name);
        }

        // Try: HKLM\SOFTWARE\Classes\MediaFoundation\Transforms\Categories\{CategoryGUID}\{GUID}
        // (This would require knowing the category, so we skip for now)

        None
    }
}

/// Helper to query FriendlyName from a registry subkey
fn _query_subkey_friendly_name(parent_key: HKEY, subkey_name: &str) -> Option<String> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, KEY_READ, REG_VALUE_TYPE, RRF_RT_REG_SZ,
    };

    let subkey_wide: Vec<u16> = subkey_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            parent_key,
            windows::core::PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return None;
        }

        let value_name: Vec<u16> = "FriendlyName"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut buffer = [0u16; 512];
        let mut buffer_size = (buffer.len() * 2) as u32;
        let mut value_type = REG_VALUE_TYPE(0);

        let result = RegGetValueW(
            hkey,
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut buffer_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_ok() {
            let len = (buffer_size as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buffer[..len]))
        } else {
            None
        }
    }
}

/// Helper: Query a string value from Windows Registry
unsafe fn query_registry_string(key_path: &str, value_name: &str) -> Option<String> {
    let key_wide: Vec<u16> = key_path.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut hkey: HKEY = HKEY::default();
    let result = RegOpenKeyExW(
        HKEY_LOCAL_MACHINE,
        PCWSTR(key_wide.as_ptr()),
        Some(0),
        KEY_READ,
        &mut hkey,
    );

    if result.is_err() {
        return None;
    }

    // Query size first
    let mut size: u32 = 0;
    let mut reg_type = REG_VALUE_TYPE(0);
    let result = RegGetValueW(
        hkey,
        PCWSTR::null(),
        PCWSTR(value_wide.as_ptr()),
        RRF_RT_REG_SZ,
        Some(&mut reg_type),
        None,
        Some(&mut size),
    );

    if result.is_err() || size == 0 {
        let _ = RegCloseKey(hkey);
        return None;
    }

    // Allocate buffer and read
    let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
    let result = RegGetValueW(
        hkey,
        PCWSTR::null(),
        PCWSTR(value_wide.as_ptr()),
        RRF_RT_REG_SZ,
        Some(&mut reg_type),
        Some(buffer.as_mut_ptr() as *mut _),
        Some(&mut size),
    );

    let _ = RegCloseKey(hkey);

    if result.is_err() {
        return None;
    }

    // Convert wide string to Rust String
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}
