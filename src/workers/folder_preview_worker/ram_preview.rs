use super::FolderPreviewData;
use crate::domain::folder_appearance::FolderAppearance;
use crate::infrastructure::disk_cache::FolderPreviewSource;
use std::path::Path;
use std::sync::Arc;

impl FolderPreviewData {
    pub fn cached_preview(&self) -> Option<CachedFolderPreview> {
        if !self.premultiplied || self.retry_after_source_change || self.rgba_data.is_empty() {
            return None;
        }
        Some(CachedFolderPreview {
            pixels: Arc::clone(&self.rgba_data),
            width: self.width,
            height: self.height,
            appearance: self.appearance,
            source: self.cache_source.clone()?,
        })
    }
}

/// Premultiplied pixels with the exact source identity that produced them.
#[derive(Clone, Debug)]
pub struct CachedFolderPreview {
    pub pixels: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub appearance: FolderAppearance,
    pub source: FolderPreviewSource,
}

impl CachedFolderPreview {
    pub fn matches_request(
        &self,
        folder: &Path,
        cover: Option<&Path>,
        bucket: u32,
        appearance: FolderAppearance,
    ) -> bool {
        self.source.matches_paths(folder, cover)
            && self.width.max(self.height) >= bucket
            && self.appearance == appearance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{File, FileTimes};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn folder_preview_ram_snapshot_requires_identity_size_and_appearance() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.jpg");
        let other = dir.path().join("other.jpg");
        std::fs::write(&cover, b"image").unwrap();
        std::fs::write(&other, b"image").unwrap();
        let source = FolderPreviewSource::capture(dir.path(), Some(&cover)).unwrap();
        let preview = CachedFolderPreview {
            pixels: Arc::new(vec![255; 16 * 16 * 4]),
            width: 16,
            height: 16,
            appearance: FolderAppearance::Original,
            source: source.clone(),
        };
        assert!(preview.matches_request(dir.path(), Some(&cover), 16, FolderAppearance::Original));
        assert!(!preview.matches_request(dir.path(), None, 16, FolderAppearance::Original));
        assert!(!preview.matches_request(dir.path(), Some(&cover), 32, FolderAppearance::Original));
        assert!(!preview.matches_request(dir.path(), Some(&other), 16, FolderAppearance::Original));
        for appearance in FolderAppearance::ALL {
            assert_eq!(
                preview.matches_request(dir.path(), Some(&cover), 16, appearance),
                appearance == FolderAppearance::Original,
            );
        }
        File::options()
            .write(true)
            .open(&cover)
            .unwrap()
            .set_times(FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(100)))
            .unwrap();
        // Display no longer waits for metadata; background validation detects it.
        assert!(preview.matches_request(dir.path(), Some(&cover), 16, FolderAppearance::Original));
        assert!(!preview.source.is_current());
    }

    #[test]
    fn folder_preview_placeholders_and_obsolete_results_are_not_retained() {
        let dir = tempfile::tempdir().unwrap();
        let mut data = FolderPreviewData {
            validation_pending: false,
            replace_existing: false,
            cache_source: None,
            retry_after_source_change: false,
            cover_path: None,
            validity: Default::default(),
            appearance: FolderAppearance::Original,
            path: dir.path().to_path_buf(),
            rgba_data: Arc::new(vec![255; 16 * 16 * 4]),
            width: 16,
            height: 16,
            premultiplied: true,
        };
        // Unsafe/unavailable-cover placeholders have pixels, but no cache source.
        assert!(data.cached_preview().is_none());
        data.cache_source = FolderPreviewSource::capture(dir.path(), None);
        assert!(data.cached_preview().is_some());
        data.retry_after_source_change = true;
        assert!(data.cached_preview().is_none());
        data.retry_after_source_change = false;
        data.premultiplied = false;
        assert!(data.cached_preview().is_none());
    }
}
