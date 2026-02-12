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

            // Only check Normal components (not Prefix like "C:" or RootDir)
            if let std::path::Component::Normal(name) = component {
                let name_str = name.to_string_lossy();

                // Block NTFS Alternate Data Streams: a colon in a normal
                // filename component indicates a hidden data stream
                // (e.g. "file.txt:hidden:$DATA").
                if name_str.contains(':') {
                    return Err(SecurityError::InvalidPath(format!(
                        "NTFS Alternate Data Stream not allowed: {}",
                        name_str
                    )));
                }

                // Block Windows reserved device names (CON, NUL, PRN, AUX,
                // COM1-9, LPT1-9).  Windows silently redirects these to
                // kernel device objects regardless of extension.
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
fn is_windows_reserved_name(name: &str) -> bool {
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
///
/// Normalizes trailing dots/spaces that Windows silently strips from filenames
/// before checking the extension. Without this, a name like `malware.exe.` would
/// yield `None` from `Path::extension()` while Windows still treats it as `.exe`.
pub fn validate_file_extension(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    // Strip trailing dots/spaces that Windows ignores but bypass extension checks.
    let normalized = file_name.trim_end_matches(['.', ' ']);
    let check_path = Path::new(normalized);

    if let Some(ext) = check_path.extension() {
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

/// Validates a UNC network path for basic safety (null bytes, path traversal).
///
/// Unlike full `sanitize_path`, this does **not** check drive letters or attempt
/// canonicalization since UNC paths (`\\server\share\...`) have no local drive prefix.
/// It still blocks the most dangerous patterns that would allow an attacker to
/// escape path boundaries.
pub fn sanitize_unc_path(path: &Path) -> Result<PathBuf, SecurityError> {
    let path_str = path.to_string_lossy();

    if path_str.contains('\0') {
        return Err(SecurityError::NullBytes(path_str.to_string()));
    }

    // Raw string check: Windows path parsing may normalize `.` and `..` away before
    // the component iterator sees them. Check the raw string for traversal patterns.
    for segment in path_str.split(&['\\', '/']) {
        if segment == ".." || segment == "." {
            return Err(SecurityError::PathTraversal(path_str.to_string()));
        }
    }

    // Also check via the typed component API as a belt-and-suspenders defense.
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(SecurityError::PathTraversal(path_str.to_string()));
        }
    }

    Ok(path.to_path_buf())
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
    fn test_blocked_extensions() {
        let config = SecurityConfig::default();

        assert!(validate_file_extension(Path::new("virus.exe"), &config).is_err());
        assert!(validate_file_extension(Path::new("script.bat"), &config).is_err());
        assert!(validate_file_extension(Path::new("document.txt"), &config).is_ok());
        assert!(validate_file_extension(Path::new("image.jpg"), &config).is_ok());
    }

    #[test]
    fn test_blocked_extensions_trailing_dots_bypass() {
        let config = SecurityConfig::default();

        // Windows strips trailing dots/spaces, so "virus.exe." is actually "virus.exe"
        assert!(validate_file_extension(Path::new("virus.exe."), &config).is_err());
        assert!(validate_file_extension(Path::new("virus.exe.."), &config).is_err());
        assert!(validate_file_extension(Path::new("virus.exe. "), &config).is_err());
        assert!(validate_file_extension(Path::new("script.bat."), &config).is_err());
        // Normal safe files should still pass
        assert!(validate_file_extension(Path::new("document.txt."), &config).is_ok());
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
        assert!(result.is_ok(), "Legitimate UNC path should pass: {:?}", result);

        let result2 = sanitize_unc_path(Path::new(r"\\192.168.1.1\share\doc.pdf"));
        assert!(result2.is_ok(), "UNC with IP should pass: {:?}", result2);
    }
}
