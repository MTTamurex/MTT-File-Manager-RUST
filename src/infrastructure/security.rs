//! Módulo de segurança para sanitização e validação de paths
//! Segue as regras do .cursorrules: sanitização de inputs externos

use std::path::{Path, PathBuf};
use thiserror::Error;

/// Erros de segurança relacionados a paths
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

/// Configuração de segurança para validação de paths
#[derive(Clone, Debug)]
pub struct SecurityConfig {
    /// Drives permitidos (ex: ["C:", "D:"])
    pub allowed_drives: Vec<String>,

    /// Permitir symlinks? (padrão: false por segurança)
    pub allow_symlinks: bool,

    /// Bloquear paths com componentes especiais (.., ., ~)
    pub block_special_components: bool,

    /// Extensões bloqueadas (ex: [".exe", ".bat"])
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

/// Sanitiza e valida um path, retornando o path canonicalizado e seguro
pub fn sanitize_path(path: &Path, config: &SecurityConfig) -> Result<PathBuf, SecurityError> {
    // 1. Verifica bytes nulos (CWE-158)
    let path_str = path.to_string_lossy();
    if path_str.contains('\0') {
        return Err(SecurityError::NullBytes(path_str.to_string()));
    }

    // 2. Tenta canonicalizar o path
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            // Se não conseguir canonicalizar, pode ser path que não existe ainda
            // (ex: para criação de novo arquivo). Nesse caso, valida o path pai.
            if let Some(parent) = path.parent() {
                if parent.exists() {
                    match parent.canonicalize() {
                        Ok(_) => {
                            // Path pai é válido, aceita o path original
                            validate_path_components(path, config)?;
                            return Ok(path.to_path_buf());
                        }
                        Err(_) => {
                            return Err(SecurityError::InvalidPath(format!(
                                "Parent directory invalid: {}",
                                e
                            )));
                        }
                    }
                }
            }
            return Err(SecurityError::InvalidPath(format!(
                "Cannot canonicalize: {}",
                e
            )));
        }
    };

    // 3. Valida componentes do path canonicalizado
    validate_path_components(&canonical, config)?;

    // 4. Verifica se está em drive permitido
    validate_drive(&canonical, config)?;

    // 5. Verifica symlinks (se configurado para bloquear)
    if !config.allow_symlinks {
        check_symlink(&canonical)?;
    }

    Ok(canonical)
}

/// Valida componentes individuais do path
fn validate_path_components(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
    if config.block_special_components {
        for component in path.components() {
            let comp_str = component.as_os_str().to_string_lossy();

            // Bloqueia path traversal
            if comp_str == ".." {
                return Err(SecurityError::PathTraversal(
                    path.to_string_lossy().to_string(),
                ));
            }

            // Bloqueia diretório corrente (pode ser usado em combinação)
            if comp_str == "." {
                return Err(SecurityError::PathTraversal(
                    path.to_string_lossy().to_string(),
                ));
            }

            // Bloqueia home directory shortcut (menos relevante no Windows)
            if comp_str == "~" {
                return Err(SecurityError::InvalidPath(
                    "Home directory shortcut not allowed".to_string(),
                ));
            }
        }
    }

    Ok(())
}

/// Valida se o path está em um drive permitido
fn validate_drive(path: &Path, config: &SecurityConfig) -> Result<(), SecurityError> {
    let path_str = path.to_string_lossy().to_uppercase();

    // Handle Windows extended-length paths (\\?\C:\...) from canonicalize()
    // These are NOT UNC paths but local paths with extended prefix
    let normalized_path = if path_str.starts_with("\\\\?\\") {
        path_str[4..].to_string() // Strip \\?\ prefix
    } else if path_str.starts_with("\\\\.\\") {
        path_str[4..].to_string() // Strip \\.\ prefix (device path)
    } else {
        path_str.clone()
    };

    // Extrai a letra do drive (ex: "C:\" -> "C")
    let drive_letter = normalized_path.chars().next().unwrap_or(' ');

    if drive_letter.is_alphabetic() {
        let drive = format!("{}:", drive_letter);
        if !config
            .allowed_drives
            .iter()
            .any(|d| d.to_uppercase() == drive)
        {
            return Err(SecurityError::OutsideAllowedDrive(normalized_path));
        }
    } else {
        // True UNC path (\\server\share) - not extended path prefix
        // Por segurança, bloqueia UNC paths a menos que explicitamente permitido
        if normalized_path.starts_with("\\\\") {
            return Err(SecurityError::OutsideAllowedDrive(
                "UNC paths not allowed".to_string(),
            ));
        }
    }

    Ok(())
}

/// Verifica se o path ou qualquer componente pai é um symlink
fn check_symlink(path: &Path) -> Result<(), SecurityError> {
    let mut current = path.to_path_buf();

    // Verifica cada componente do path
    while current.exists() {
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(SecurityError::SymlinkDetected(
                    current.to_string_lossy().to_string(),
                ));
            }
        }

        // Sobe para o diretório pai
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Ok(())
}

/// Valida extensão de arquivo contra lista de extensões bloqueadas
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

/// Função helper para sanitização rápida (usa configuração padrão)
pub fn sanitize_path_quick(path: &Path) -> Result<PathBuf, SecurityError> {
    let config = SecurityConfig::default();
    sanitize_path(path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_path_traversal_blocked() {
        let config = SecurityConfig::default();

        // Path traversal deve ser bloqueado
        assert!(sanitize_path(Path::new("C:\\Windows\\..\\System32"), &config).is_err());
        assert!(sanitize_path(Path::new("..\\secret.txt"), &config).is_err());
        assert!(sanitize_path(Path::new(".\\..\\escape"), &config).is_err());
    }

    #[test]
    fn test_valid_paths_allowed() {
        // Use a permissive config for testing since temp directories
        // on Windows may contain junction points (e.g., AppData\Local\Temp)
        let config = SecurityConfig {
            allow_symlinks: true, // Temp paths may contain junction points
            ..SecurityConfig::default()
        };

        // Paths válidos devem passar (usa paths que existem no sistema)
        let temp_dir = tempdir().unwrap();
        let test_file = temp_dir.path().join("test.txt");
        fs::write(&test_file, "test").unwrap();

        let result = sanitize_path(&test_file, &config);
        if let Err(e) = &result {
            eprintln!("test_file path: {:?}", test_file);
            eprintln!("Error: {:?}", e);
        }
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
    fn test_symlink_detection() {
        let temp_dir = tempdir().unwrap();
        let real_file = temp_dir.path().join("real.txt");
        let link_file = temp_dir.path().join("link.txt");

        fs::write(&real_file, "content").unwrap();

        // Creating symlinks on Windows requires elevated privileges (SeCreateSymbolicLinkPrivilege)
        // Skip this test if we don't have the required permissions
        #[cfg(windows)]
        {
            match std::os::windows::fs::symlink_file(&real_file, &link_file) {
                Ok(_) => {
                    let mut config = SecurityConfig::default();
                    config.allow_symlinks = false;
                    assert!(sanitize_path(&link_file, &config).is_err());

                    config.allow_symlinks = true;
                    assert!(sanitize_path(&link_file, &config).is_ok());
                }
                Err(e) if e.raw_os_error() == Some(1314) => {
                    // ERROR_PRIVILEGE_NOT_HELD - skip test gracefully
                    eprintln!("Skipping symlink test: requires elevated privileges");
                }
                Err(e) => panic!("Unexpected error creating symlink: {}", e),
            }
        }

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real_file, &link_file).unwrap();

            let mut config = SecurityConfig::default();
            config.allow_symlinks = false;
            assert!(sanitize_path(&link_file, &config).is_err());

            config.allow_symlinks = true;
            assert!(sanitize_path(&link_file, &config).is_ok());
        }
    }
}
