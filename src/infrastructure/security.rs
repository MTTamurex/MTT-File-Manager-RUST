//! Security module for path sanitization and validation.
//! Applies defensive checks for path traversal, invalid prefixes and symlinks.

use std::path::{Path, PathBuf};
use thiserror::Error;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

/// Path-related security errors.
#[derive(Error, Debug)]
pub enum SecurityError {
    #[error("Path traversal attempt detected: {0}")]
    PathTraversal(String),

    #[error("Path outside allowed drives: {0}")]
    OutsideAllowedDrive(String),

    #[error("Invalid or malformed path: {0}")]
    InvalidPath(String),

    #[error("Symlink detected (not allowed): {0}")]
    SymlinkDetected(String),

    #[error("Path contains null bytes: {0}")]
    NullBytes(String),
}

/// Security configuration for path validation.
#[derive(Clone, Debug)]
pub struct SecurityConfig {
    /// Allowed drives (example: ["C:", "D:"]).
    pub allowed_drives: Vec<String>,

    /// Allow symlinks? Default: false.
    pub allow_symlinks: bool,

    /// Block special components (`..`, `.`, `~`).
    pub block_special_components: bool,

    /// Blocked file extensions (example: [".exe", ".bat"]).
    pub blocked_extensions: Vec<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_drives: vec![
                "C:".to_string(),
                "D:".to_string(),
                "E:".to_string(),
                "F:".to_string(),
                "G:".to_string(),
            ],
            allow_symlinks: false,
            block_special_components: true,
            blocked_extensions: vec![
                ".exe".to_string(),
                ".bat".to_string(),
                ".cmd".to_string(),
                ".ps1".to_string(),
                ".vbs".to_string(),
                ".js".to_string(),
            ],
        }
    }
}

/// Sanitize and validate a path, returning a canonicalized safe path.
pub fn sanitize_path(path: &Path, config: &SecurityConfig) -> Result<PathBuf, SecurityError> {
    let path_str = path.to_string_lossy();
    if path_str.contains('\0') {
        return Err(SecurityError::NullBytes(path_str.to_string()));
    }

    // Validate user-provided components before canonicalization so patterns like
    // `..\` are blocked even when canonicalization would normalize them away.
    validate_path_components(path, config)?;

    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            // Fallback for paths that do not exist yet (e.g. file creation):
            // validate against the canonical parent and rebuild an absolute path.
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    let canonical_parent = parent.canonicalize().map_err(|_| {
                        SecurityError::InvalidPath(format!("Parent directory invalid: {}", e))
                    })?;

                    validate_drive(&canonical_parent, config)?;
                    if !config.allow_symlinks {
                        check_symlink(&canonical_parent)?;
                    }

                    let fallback = match path.file_name() {
                        Some(name) => canonical_parent.join(name),
                        None => canonical_parent,
                    };
                    return Ok(normalize_for_shell_apis(&fallback));
                }
            }

            return Err(SecurityError::InvalidPath(format!(
                "Cannot canonicalize: {}",
                e
            )));
        }
    };

    validate_path_components(&canonical, config)?;
    validate_drive(&canonical, config)?;
    if !config.allow_symlinks {
        check_symlink(&canonical)?;
    }

    Ok(normalize_for_shell_apis(&canonical))
}

fn extract_local_drive(path: &Path) -> Option<String> {
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

fn drive_is_allowed(drive: &str, config: &SecurityConfig) -> bool {
    config
        .allowed_drives
        .iter()
        .any(|allowed| allowed.to_uppercase() == drive)
}

fn has_relative_components(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    })
}

/// Sanitizes a path and falls back to lexical validation for absolute local-drive paths.
///
/// This is useful for virtual drives where canonicalization can resolve to provider/UNC paths
/// that fail strict drive validation despite the original path being a valid `X:\...` input.
pub fn sanitize_path_with_local_drive_fallback(
    path: &Path,
    config: &SecurityConfig,
) -> Result<PathBuf, SecurityError> {
    match sanitize_path(path, config) {
        Ok(valid) => Ok(valid),
        Err(original_error) => {
            let path_str = path.to_string_lossy();
            if path_str.contains('\0') {
                return Err(SecurityError::NullBytes(path_str.to_string()));
            }

            let Some(drive) = extract_local_drive(path) else {
                return Err(original_error);
            };

            if !drive_is_allowed(&drive, config) {
                return Err(SecurityError::OutsideAllowedDrive(path_str.to_string()));
            }

            if config.block_special_components && has_relative_components(path) {
                return Err(SecurityError::PathTraversal(path_str.to_string()));
            }

            if !config.allow_symlinks {
                check_symlink(path)?;
            }

            Ok(normalize_for_shell_apis(path))
        }
    }
}

/// Validate each path component.
fn validate_path_components(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
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
        }
    }

    Ok(())
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
///
/// `std::fs::canonicalize` commonly returns paths like `\\?\C:\...`.
/// Many shell operations (SHFileOperation / parsing-name based APIs) are
/// more reliable with the regular `C:\...` representation.
fn normalize_for_shell_apis(path: &Path) -> PathBuf {
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
fn validate_drive(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
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

/// Check if path or any parent component is a symlink.
fn check_symlink(path: &Path) -> Result<(), SecurityError> {
    let mut current = path.to_path_buf();

    while current.exists() {
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if is_link_like_path(&metadata) {
                return Err(SecurityError::SymlinkDetected(
                    current.to_string_lossy().to_string(),
                ));
            }
        }

        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Ok(())
}

#[cfg(windows)]
#[inline]
fn is_link_like_path(metadata: &std::fs::Metadata) -> bool {
    // Windows reparse points include symlinks, junctions and mount points.
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
#[inline]
fn is_link_like_path(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

/// Validate file extension against blocked list.
pub fn validate_file_extension(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
    if let Some(ext) = path.extension() {
        let ext_str = ext.to_string_lossy().to_lowercase();
        let ext_with_dot = format!(".{}", ext_str);

        if config.blocked_extensions.iter().any(|blocked| {
            blocked.to_lowercase() == ext_str || blocked.to_lowercase() == ext_with_dot
        }) {
            return Err(SecurityError::InvalidPath(format!(
                "Blocked file extension: .{}",
                ext_str
            )));
        }
    }

    Ok(())
}

/// Quick helper using default security config.
pub fn sanitize_path_quick(path: &Path) -> Result<PathBuf, SecurityError> {
    sanitize_path(path, &SecurityConfig::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_path_traversal_blocked() {
        let config = SecurityConfig::default();

        assert!(sanitize_path(Path::new("C:\\Windows\\..\\System32"), &config).is_err());
        assert!(sanitize_path(Path::new("..\\secret.txt"), &config).is_err());
        assert!(sanitize_path(Path::new(".\\..\\escape"), &config).is_err());
    }

    #[test]
    fn test_valid_paths_allowed() {
        // Use permissive symlink mode for temp paths because on Windows temp
        // directories may include junctions.
        let config = SecurityConfig {
            allow_symlinks: true,
            ..SecurityConfig::default()
        };

        let temp_dir = tempdir().expect("temp dir");
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "test").expect("write temp file");

        let result = sanitize_path(&test_file, &config);
        assert!(result.is_ok(), "Expected OK, got: {:?}", result);
        assert!(sanitize_path(temp_dir.path(), &config).is_ok());
    }

    #[test]
    fn test_blocked_extensions() {
        let config = SecurityConfig::default();

        assert!(validate_file_extension(Path::new("virus.exe"), &config).is_err());
        assert!(validate_file_extension(Path::new("script.bat"), &config).is_err());
        assert!(validate_file_extension(Path::new("document.txt"), &config).is_ok());
        assert!(validate_file_extension(Path::new("image.jpg"), &config).is_ok());
    }

    #[test]
    fn test_normalize_for_shell_apis_strips_verbatim_prefixes() {
        let local = PathBuf::from(r"\\?\C:\Temp\file.txt");
        let unc = PathBuf::from(r"\\?\UNC\server\share\file.txt");
        let device = PathBuf::from(r"\\.\C:\Temp\file.txt");

        assert_eq!(
            normalize_for_shell_apis(&local),
            PathBuf::from(r"C:\Temp\file.txt")
        );
        assert_eq!(
            normalize_for_shell_apis(&unc),
            PathBuf::from(r"\\server\share\file.txt")
        );
        assert_eq!(
            normalize_for_shell_apis(&device),
            PathBuf::from(r"C:\Temp\file.txt")
        );
    }

    #[test]
    fn test_local_drive_fallback_allows_virtual_drive_style_paths() {
        let config = SecurityConfig {
            allowed_drives: vec!["Z:".to_string()],
            allow_symlinks: true,
            ..SecurityConfig::default()
        };

        let result =
            sanitize_path_with_local_drive_fallback(Path::new(r"Z:\vault\file.txt"), &config);
        assert!(
            result.is_ok(),
            "Expected fallback to accept local drive path"
        );
        assert_eq!(result.unwrap(), PathBuf::from(r"Z:\vault\file.txt"));
    }

    #[test]
    fn test_local_drive_fallback_blocks_relative_components() {
        let config = SecurityConfig {
            allowed_drives: vec!["Z:".to_string()],
            allow_symlinks: true,
            ..SecurityConfig::default()
        };

        let result =
            sanitize_path_with_local_drive_fallback(Path::new(r"Z:\vault\..\secret.txt"), &config);
        assert!(result.is_err(), "Path traversal should stay blocked");
    }

    #[test]
    fn test_local_drive_fallback_respects_allowed_drives() {
        let config = SecurityConfig {
            allowed_drives: vec!["C:".to_string()],
            allow_symlinks: true,
            ..SecurityConfig::default()
        };

        let result =
            sanitize_path_with_local_drive_fallback(Path::new(r"Z:\vault\file.txt"), &config);
        assert!(result.is_err(), "Drive outside allow list must be blocked");
    }

    #[test]
    fn test_symlink_detection() {
        let temp_dir = tempdir().expect("temp dir");
        let real_file = temp_dir.path().join("real.txt");
        let link_file = temp_dir.path().join("link.txt");

        fs::write(&real_file, "content").expect("write real file");

        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_file(&real_file, &link_file) {
                Ok(_) => {
                    let config_block = SecurityConfig {
                        allow_symlinks: false,
                        ..SecurityConfig::default()
                    };
                    assert!(sanitize_path(&link_file, &config_block).is_err());

                    let config_allow = SecurityConfig {
                        allow_symlinks: true,
                        ..SecurityConfig::default()
                    };
                    assert!(sanitize_path(&link_file, &config_allow).is_ok());
                }
                Err(e) if e.raw_os_error() == Some(1314) => {
                    // ERROR_PRIVILEGE_NOT_HELD - skip gracefully.
                    eprintln!("Skipping symlink test: requires elevated privileges");
                }
                Err(e) => panic!("Unexpected symlink creation error: {}", e),
            }
        }

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &link_file).expect("create symlink");

            let config_block = SecurityConfig {
                allow_symlinks: false,
                ..SecurityConfig::default()
            };
            assert!(sanitize_path(&link_file, &config_block).is_err());

            let config_allow = SecurityConfig {
                allow_symlinks: true,
                ..SecurityConfig::default()
            };
            assert!(sanitize_path(&link_file, &config_allow).is_ok());
        }
    }
}
