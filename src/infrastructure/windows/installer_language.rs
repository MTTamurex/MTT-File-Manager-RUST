use windows::{
    core::PCWSTR,
    Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_VALUE_TYPE, RRF_RT_REG_SZ,
    },
};

/// Reads the language selected during installation from the registry.
///
/// The Inno Setup installer writes `HKLM\SOFTWARE\MTT-File-Manager\InstallerLanguage`
/// during `ssPostInstall`. The app reads this value on first launch to determine
/// the initial UI language, then persists it to SQLite for subsequent launches.
///
/// Returns `None` if the key does not exist (e.g. portable use, not installed).
/// The app must NEVER attempt to delete this key — only the admin-elevated
/// installer/uninstaller can modify `HKLM`.
pub fn read_installer_language() -> Option<String> {
    unsafe {
        let key_wide: Vec<u16> = "SOFTWARE\\MTT-File-Manager"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let value_wide: Vec<u16> = "InstallerLanguage"
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

        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }
}
