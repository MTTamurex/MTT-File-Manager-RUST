use std::path::Path;

use super::SecurityError;

/// Check if path or any parent component is a symlink/reparse point.
pub(super) fn check_symlink(path: &Path) -> Result<(), SecurityError> {
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
    use std::os::windows::fs::MetadataExt;

    // Windows reparse points include symlinks, junctions and mount points.
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0
}

#[cfg(not(windows))]
#[inline]
fn is_link_like_path(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}
