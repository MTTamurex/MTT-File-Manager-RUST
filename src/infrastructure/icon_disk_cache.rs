//! Persistent disk cache for shell icons.
//!
//! Stores raw RGBA pixel data per extension so that subsequent app launches
//! can hydrate the `extension_cache` lazily without calling `SHGetFileInfoW`.
//! Unique per-file icons are stored separately in SQLite as lossless PNG blobs.
//!
//! File format per extension: `{ext}.rgba`
//!   [width: u32 LE][height: u32 LE][rgba_pixels...]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod file_icons;
pub use file_icons::FileIconCacheKey;
use rusqlite::Connection;

const RGBA_HEADER_LEN: usize = 8;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const MAX_ICON_DIMENSION: u32 = 512;
const MAX_ICON_RGBA_BYTES: usize =
    (MAX_ICON_DIMENSION as usize) * (MAX_ICON_DIMENSION as usize) * RGBA_BYTES_PER_PIXEL;
const MAX_ICON_CACHE_FILE_BYTES: u64 = (RGBA_HEADER_LEN + MAX_ICON_RGBA_BYTES) as u64;

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

fn read_cache_file(path: &Path) -> Option<Vec<u8>> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_ICON_CACHE_FILE_BYTES => {}
        _ => {
            let _ = std::fs::remove_file(path);
            return None;
        }
    }

    std::fs::read(path).ok()
}

fn parse_cached_icon(mut data: Vec<u8>) -> Option<(Vec<u8>, u32, u32)> {
    if data.len() < RGBA_HEADER_LEN {
        return None;
    }

    let width = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let height = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let expected = expected_rgba_len(width, height)?;
    let total_len = RGBA_HEADER_LEN.checked_add(expected)?;
    if data.len() != total_len {
        return None;
    }

    drop(data.drain(..RGBA_HEADER_LEN));
    Some((data, width, height))
}

/// On-disk cache for extension → RGBA icon data and per-file unique icons.
pub struct IconDiskCache {
    extension_dir: PathBuf,
    pub(super) file_icon_db: Mutex<Connection>,
    pub(super) file_icon_trim_lock: Mutex<()>,
}

impl IconDiskCache {
    /// Create (or open) the icon disk cache directory.
    pub fn new(app_data_dir: &Path) -> Self {
        let extension_dir = app_data_dir.join("extension_icons");
        if let Err(e) = std::fs::create_dir_all(&extension_dir) {
            log::warn!(
                "[IconDiskCache] Failed to create dir {:?}: {}",
                extension_dir,
                e
            );
        }

        let file_icon_db = file_icons::open_file_icon_db(app_data_dir);

        Self {
            extension_dir,
            file_icon_db: Mutex::new(file_icon_db),
            file_icon_trim_lock: Mutex::new(()),
        }
    }

    /// Lazily load a single extension's cached icon from disk on demand.
    ///
    /// Returns `Some((pixels, width, height))` when a valid file exists for
    /// the canonical extension, `None` otherwise. Invalid/stale files found
    /// during the read are removed so the caller can fall back to a fresh
    /// Shell extraction.
    ///
    /// This avoids the boot-time `load_all()` walk that materialises every
    /// cached icon (each Jumbo entry is 256 KB of RGBA) into a permanent
    /// in-process `HashMap` even for extensions the user may never view in
    /// the current session.
    pub fn load_one(&self, ext: &str) -> Option<(Vec<u8>, u32, u32)> {
        if ext.is_empty() {
            return None;
        }
        let ext_lower = ext.to_lowercase();
        let canonical = crate::infrastructure::windows::icons::canonical_icon_ext(&ext_lower);
        if canonical != ext_lower {
            let _ = std::fs::remove_file(self.extension_dir.join(format!("{}.rgba", ext_lower)));
        }
        if crate::infrastructure::windows::icons::requires_real_file_for_shared_icon(canonical) {
            return None;
        }
        let path = self
            .extension_dir
            .join(format!("{}.rgba", canonical.to_lowercase()));
        let Some((pixels, width, height)) = read_cache_file(&path).and_then(parse_cached_icon)
        else {
            let _ = std::fs::remove_file(&path);
            return None;
        };
        Some((pixels, width, height))
    }

    /// Save an extension's icon data to disk.
    /// Called from worker threads after extracting a new extension icon.
    pub fn save(&self, ext: &str, pixels: &[u8], width: u32, height: u32) {
        if ext.is_empty()
            || pixels.is_empty()
            || expected_rgba_len(width, height) != Some(pixels.len())
        {
            return;
        }
        // Always save under the canonical extension so mapped types (sys→dll)
        // share a single cache file.
        let canonical = crate::infrastructure::windows::icons::canonical_icon_ext(ext);
        if crate::infrastructure::windows::icons::requires_real_file_for_shared_icon(canonical) {
            return;
        }
        let path = self
            .extension_dir
            .join(format!("{}.rgba", canonical.to_lowercase()));
        // Don't overwrite if already exists (another worker may have written it).
        if path.exists() {
            return;
        }
        let mut data = Vec::with_capacity(8 + pixels.len());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(pixels);
        if let Err(e) = std::fs::write(&path, &data) {
            log::warn!("[IconDiskCache] Failed to write {:?}: {}", path, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_blob(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut data = Vec::with_capacity(RGBA_HEADER_LEN + pixels.len());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(pixels);
        data
    }

    #[test]
    fn parse_cached_icon_accepts_valid_rgba() {
        let pixels = vec![7; 2 * 3 * RGBA_BYTES_PER_PIXEL];
        let parsed = parse_cached_icon(cache_blob(2, 3, &pixels)).unwrap();

        assert_eq!(parsed.0, pixels);
        assert_eq!(parsed.1, 2);
        assert_eq!(parsed.2, 3);
    }

    #[test]
    fn parse_cached_icon_rejects_size_mismatch() {
        let pixels = vec![7; 7];

        assert!(parse_cached_icon(cache_blob(2, 2, &pixels)).is_none());
    }

    #[test]
    fn expected_rgba_len_rejects_zero_and_oversized_dimensions() {
        assert_eq!(expected_rgba_len(0, 1), None);
        assert_eq!(expected_rgba_len(1, 0), None);
        assert_eq!(expected_rgba_len(MAX_ICON_DIMENSION + 1, 1), None);
        assert_eq!(expected_rgba_len(1, MAX_ICON_DIMENSION + 1), None);
        assert_eq!(expected_rgba_len(u32::MAX, u32::MAX), None);
    }
}
