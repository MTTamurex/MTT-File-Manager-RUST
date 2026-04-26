//! Security module for path sanitization and validation.
//! Applies defensive checks for path traversal, invalid prefixes and symlinks.

use std::path::{Path, PathBuf};
use thiserror::Error;

mod components;
mod drive;
mod shell_namespace;
mod symlink;
mod unc;

pub use shell_namespace::{
    classify_shell_namespace_path, classify_shell_namespace_str, ShellNamespacePathKind,
};

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
    components::validate_path_components(path, config)?;

    // SEC: Check for reparse points (junctions/symlinks/mount points) on the
    // ORIGINAL path BEFORE canonicalization. `Path::canonicalize` resolves
    // junctions silently via GetFinalPathNameByHandle, which would otherwise
    // let an attacker hide a junction inside e.g. `D:\public\link → C:\Windows\System32`
    // and trick the post-canonicalization check (the canonical target itself
    // is not a reparse point, so the old order accepted it).
    if !config.allow_symlinks {
        symlink::check_symlink(path)?;
    }

    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            // Fallback for paths that do not exist yet (e.g. file creation):
            // validate against canonical parent and rebuild an absolute path.
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    let canonical_parent = parent.canonicalize().map_err(|_| {
                        SecurityError::InvalidPath(format!("Parent directory invalid: {}", e))
                    })?;

                    drive::validate_drive(&canonical_parent, config)?;
                    if !config.allow_symlinks {
                        symlink::check_symlink(&canonical_parent)?;
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

    components::validate_path_components(&canonical, config)?;
    drive::validate_drive(&canonical, config)?;
    if !config.allow_symlinks {
        symlink::check_symlink(&canonical)?;
    }

    Ok(normalize_for_shell_apis(&canonical))
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

            let Some(drive) = drive::extract_local_drive(path) else {
                return Err(original_error);
            };

            if !drive::drive_is_allowed(&drive, config) {
                return Err(SecurityError::OutsideAllowedDrive(path_str.to_string()));
            }

            if config.block_special_components && drive::has_relative_components(path) {
                return Err(SecurityError::PathTraversal(path_str.to_string()));
            }

            if !config.allow_symlinks {
                symlink::check_symlink(path)?;
            }

            Ok(normalize_for_shell_apis(path))
        }
    }
}

/// Returns true if `name` is a Windows reserved device name.
pub fn is_windows_reserved_name(name: &str) -> bool {
    components::is_windows_reserved_name(name)
}

/// Converts Windows verbatim prefixes to regular paths for Shell APIs.
fn normalize_for_shell_apis(path: &Path) -> PathBuf {
    drive::normalize_for_shell_apis(path)
}

/// Quick helper using default security config.
pub fn sanitize_path_quick(path: &Path) -> Result<PathBuf, SecurityError> {
    sanitize_path(path, &SecurityConfig::default())
}

/// Validates a UNC network path for basic safety (null bytes, path traversal).
pub fn sanitize_unc_path(path: &Path) -> Result<PathBuf, SecurityError> {
    unc::sanitize_unc_path(path)
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
    fn test_ads_blocked() {
        let config = SecurityConfig::default();

        assert!(sanitize_path(Path::new("C:\\temp\\file.txt:hidden"), &config).is_err());
        assert!(sanitize_path(Path::new("C:\\temp\\file.txt:evil:$DATA"), &config).is_err());
    }

    #[test]
    fn test_reserved_names_blocked() {
        let config = SecurityConfig::default();

        assert!(sanitize_path(Path::new("C:\\temp\\CON"), &config).is_err());
        assert!(sanitize_path(Path::new("C:\\temp\\NUL"), &config).is_err());
        assert!(sanitize_path(Path::new("C:\\temp\\COM1"), &config).is_err());
        assert!(sanitize_path(Path::new("C:\\temp\\LPT1.txt"), &config).is_err());
        assert!(sanitize_path(Path::new("C:\\temp\\PRN"), &config).is_err());
        assert!(sanitize_path(Path::new("C:\\temp\\AUX"), &config).is_err());
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
                    log::warn!("Skipping symlink test: requires elevated privileges");
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

    #[test]
    fn test_unc_path_traversal_blocked() {
        assert!(sanitize_unc_path(Path::new(r"\\server\share\..\secret")).is_err());
        assert!(sanitize_unc_path(Path::new(r"\\server\share\.\hidden")).is_err());
        assert!(sanitize_unc_path(Path::new(r"\\evil\share\..\..\windows\system32")).is_err());
    }

    #[test]
    fn test_unc_path_null_bytes_blocked() {
        assert!(sanitize_unc_path(Path::new("\\\\server\\share\\file\0.txt")).is_err());
    }

    #[test]
    fn test_unc_path_valid_allowed() {
        let result = sanitize_unc_path(Path::new(r"\\server\share\folder\file.txt"));
        assert!(
            result.is_ok(),
            "Legitimate UNC path should pass: {:?}",
            result
        );

        let result2 = sanitize_unc_path(Path::new(r"\\192.168.1.1\share\doc.pdf"));
        assert!(result2.is_ok(), "UNC with IP should pass: {:?}", result2);
    }
}
