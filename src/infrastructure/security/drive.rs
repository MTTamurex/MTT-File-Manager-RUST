use std::path::{Path, PathBuf};

use super::{SecurityConfig, SecurityError};

pub(super) fn extract_local_drive(path: &Path) -> Option<String> {
    let raw = path.to_string_lossy();
    let normalized = if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        stripped
    } else if let Some(stripped) = raw.strip_prefix(r"\\.\") {
        stripped
    } else {
        &raw
    };

    let bytes = normalized.as_bytes();
    if bytes.len() < 3 {
        return None;
    }

    if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        Some(format!("{}:", (bytes[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

pub(super) fn drive_is_allowed(drive: &str, config: &SecurityConfig) -> bool {
    config
        .allowed_drives
        .iter()
        .any(|allowed| allowed.to_uppercase() == drive)
}

pub(super) fn has_relative_components(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    })
}

fn normalize_windows_prefix(path_upper: &str) -> String {
    if let Some(stripped) = path_upper.strip_prefix("\\\\?\\") {
        stripped.to_string()
    } else if let Some(stripped) = path_upper.strip_prefix("\\\\.\\") {
        stripped.to_string()
    } else {
        path_upper.to_string()
    }
}

/// Converts Windows verbatim prefixes to regular paths for Shell APIs.
pub(super) fn normalize_for_shell_apis(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();

    if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", stripped));
    }
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(stripped);
    }
    if let Some(stripped) = s.strip_prefix(r"\\.\") {
        return PathBuf::from(stripped);
    }

    path.to_path_buf()
}

/// Validate that a path is inside an allowed drive.
pub(super) fn validate_drive(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
    let path_upper = path.to_string_lossy().to_uppercase();
    let normalized = normalize_windows_prefix(&path_upper);

    // Extended UNC format: \\?\UNC\server\share\...
    if normalized.starts_with("UNC\\") || normalized.starts_with("\\\\") {
        return Err(SecurityError::OutsideAllowedDrive(
            "UNC paths not allowed".to_string(),
        ));
    }

    let bytes = normalized.as_bytes();
    if bytes.len() < 2 || bytes[1] != b':' || !(bytes[0] as char).is_ascii_alphabetic() {
        return Err(SecurityError::InvalidPath(format!(
            "Invalid drive prefix: {}",
            normalized
        )));
    }

    let drive = format!("{}:", (bytes[0] as char).to_ascii_uppercase());
    if !config
        .allowed_drives
        .iter()
        .any(|d| d.to_uppercase() == drive)
    {
        return Err(SecurityError::OutsideAllowedDrive(normalized));
    }

    Ok(())
}
