//! Persistent disk cache for unique per-file icons.
//!
//! Only file types whose icon can vary per file (programs, shortcuts, `.ico`,
//! etc.) are persisted. Drive, shell namespace, special folder, and shared
//! extension icons intentionally remain session-only so they always follow the
//! current Windows Shell state on the next launch.

use parking_lot::Mutex;
use std::path::Path;

mod file_icons;
pub use file_icons::FileIconCacheKey;
use rusqlite::Connection;

const RGBA_BYTES_PER_PIXEL: usize = 4;
const MAX_ICON_DIMENSION: u32 = 512;
const MAX_ICON_RGBA_BYTES: usize =
    (MAX_ICON_DIMENSION as usize) * (MAX_ICON_DIMENSION as usize) * RGBA_BYTES_PER_PIXEL;

pub(super) fn expected_rgba_len(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_ICON_DIMENSION || height > MAX_ICON_DIMENSION {
        return None;
    }

    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let len = width
        .checked_mul(height)?
        .checked_mul(RGBA_BYTES_PER_PIXEL)?;
    (len <= MAX_ICON_RGBA_BYTES).then_some(len)
}

/// On-disk cache for per-file unique icons.
pub struct IconDiskCache {
    pub(super) file_icon_db: Mutex<Connection>,
    pub(super) file_icon_trim_lock: Mutex<()>,
}

impl IconDiskCache {
    /// Create (or open) the icon disk cache database.
    pub fn new(app_data_dir: &Path) -> Self {
        let legacy_extension_dir = app_data_dir.join("extension_icons");
        if legacy_extension_dir.exists() {
            if let Err(error) = std::fs::remove_dir_all(&legacy_extension_dir) {
                log::warn!(
                    "[IconDiskCache] Failed to remove legacy extension icon cache {:?}: {}",
                    legacy_extension_dir,
                    error
                );
            }
        }

        let file_icon_db = file_icons::open_file_icon_db(app_data_dir);

        Self {
            file_icon_db: Mutex::new(file_icon_db),
            file_icon_trim_lock: Mutex::new(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_rgba_len_rejects_zero_and_oversized_dimensions() {
        assert_eq!(expected_rgba_len(0, 1), None);
        assert_eq!(expected_rgba_len(1, 0), None);
        assert_eq!(expected_rgba_len(MAX_ICON_DIMENSION + 1, 1), None);
        assert_eq!(expected_rgba_len(1, MAX_ICON_DIMENSION + 1), None);
        assert_eq!(expected_rgba_len(u32::MAX, u32::MAX), None);
    }
}
