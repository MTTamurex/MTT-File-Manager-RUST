use std::path::{Path, PathBuf};

use super::SecurityError;

/// Validates a UNC network path for basic safety (null bytes, path traversal).
pub(super) fn sanitize_unc_path(path: &Path) -> Result<PathBuf, SecurityError> {
    let path_str = path.to_string_lossy();

    if path_str.contains('\0') {
        return Err(SecurityError::NullBytes(path_str.to_string()));
    }

    // Raw string check for traversal patterns.
    for segment in path_str.split(&['\\', '/']) {
        if segment == ".." || segment == "." {
            return Err(SecurityError::PathTraversal(path_str.to_string()));
        }
    }

    // Typed component check as defense in depth.
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
