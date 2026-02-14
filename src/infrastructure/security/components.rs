use std::path::Path;

use super::{SecurityConfig, SecurityError};

/// Normalize a string to Unicode NFC (Normalization Form Composed) using
/// Windows NormalizeString. Returns the input unchanged on failure.
#[cfg(windows)]
fn normalize_nfc(s: &str) -> String {
    if s.is_ascii() {
        return s.to_string();
    }

    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Globalization::{NormalizeString, NORM_FORM};

    const NFC: NORM_FORM = NORM_FORM(1);
    let wide: Vec<u16> = std::ffi::OsStr::new(s).encode_wide().collect();

    unsafe {
        let needed = NormalizeString(NFC, &wide, None);
        if needed <= 0 {
            return s.to_string();
        }

        let mut buf = vec![0u16; needed as usize];
        let actual = NormalizeString(NFC, &wide, Some(&mut buf));
        if actual <= 0 {
            return s.to_string();
        }

        String::from_utf16_lossy(&buf[..actual as usize])
    }
}

#[cfg(not(windows))]
fn normalize_nfc(s: &str) -> String {
    s.to_string()
}

/// Validate each path component.
pub(super) fn validate_path_components(
    path: &Path,
    config: &SecurityConfig,
) -> Result<(), SecurityError> {
    if config.block_special_components {
        for component in path.components() {
            let comp_str = component.as_os_str().to_string_lossy();

            if comp_str == ".." || comp_str == "." {
                return Err(SecurityError::PathTraversal(
                    path.to_string_lossy().to_string(),
                ));
            }

            if comp_str == "~" {
                return Err(SecurityError::InvalidPath(
                    "Home directory shortcut not allowed".to_string(),
                ));
            }

            // Only check normal components (not prefix/root).
            if let std::path::Component::Normal(name) = component {
                let name_str = normalize_nfc(&name.to_string_lossy());

                // Block NTFS Alternate Data Streams in path components.
                if name_str.contains(':') {
                    return Err(SecurityError::InvalidPath(format!(
                        "NTFS Alternate Data Stream not allowed: {}",
                        name_str
                    )));
                }

                // Block Windows reserved device names.
                let base_name = name_str.split('.').next().unwrap_or("");
                if is_windows_reserved_name(base_name) {
                    return Err(SecurityError::InvalidPath(format!(
                        "Windows reserved device name not allowed: {}",
                        name_str
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Returns true if `name` is a Windows reserved device name.
pub(super) fn is_windows_reserved_name(name: &str) -> bool {
    let upper = name.to_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}
