use super::CachedFolderPreview;
use crate::domain::folder_appearance::FolderAppearance;
use crate::domain::folder_preview_validity::FolderPreviewValidity;
use crate::infrastructure::disk_cache::FolderPreviewSource;
use std::path::PathBuf;
use std::sync::Arc;

pub struct FolderPreviewData {
    /// Placeholders never carry a reusable source identity.
    pub cache_source: Option<FolderPreviewSource>,
    /// Cached artwork is displayed while its source is validated separately.
    pub validation_pending: bool,
    /// Terminal background validation (possibly empty). Never finishes a newer
    /// foreground request; corrected pixels replace the old image at equal size.
    pub replace_existing: bool,
    pub retry_after_source_change: bool,
    pub cover_path: Option<PathBuf>,
    pub validity: FolderPreviewValidity,
    pub appearance: FolderAppearance,
    pub path: PathBuf,
    pub rgba_data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub premultiplied: bool,
}

impl FolderPreviewData {
    pub fn should_keep_texture(&self, size: [usize; 2], force_replace: bool) -> bool {
        size[0] >= self.width as usize
            && size[1] >= self.height as usize
            && !force_replace
            && !self.replace_existing
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FolderPreviewRequestMode {
    Load,
    Refresh,
    ValidateCached,
    RecomposeIfCacheMiss,
}

#[derive(Clone, Debug)]
pub struct FolderPreviewRequest {
    pub(super) cached_preview: Option<CachedFolderPreview>,
    pub(super) validation_source: Option<FolderPreviewSource>,
    pub(super) validity: FolderPreviewValidity,
    pub path: PathBuf,
    pub size_px: u32,
    pub cover_path: Option<PathBuf>,
    pub(super) mode: FolderPreviewRequestMode,
}

impl FolderPreviewRequest {
    pub fn with_cached_preview(mut self, preview: Option<CachedFolderPreview>) -> Self {
        self.cached_preview = preview;
        self
    }

    pub fn with_validity(mut self, validity: FolderPreviewValidity) -> Self {
        self.validity = validity;
        self
    }

    pub fn require_validation(mut self, required: bool) -> Self {
        if required {
            self.mode = FolderPreviewRequestMode::Refresh;
            self.cached_preview = None;
        }
        self
    }

    pub fn new(path: PathBuf, size_px: u32, cover_path: Option<PathBuf>) -> Self {
        Self {
            cached_preview: None,
            validation_source: None,
            validity: Default::default(),
            path,
            size_px,
            cover_path,
            mode: FolderPreviewRequestMode::Load,
        }
    }

    pub fn recompose_if_cache_miss(
        path: PathBuf,
        size_px: u32,
        cover_path: Option<PathBuf>,
    ) -> Self {
        Self {
            mode: FolderPreviewRequestMode::RecomposeIfCacheMiss,
            ..Self::new(path, size_px, cover_path)
        }
    }

    pub(super) fn empty_result(&self, appearance: FolderAppearance) -> FolderPreviewData {
        FolderPreviewData {
            cache_source: None,
            validation_pending: false,
            replace_existing: false,
            retry_after_source_change: false,
            cover_path: self.cover_path.clone(),
            validity: self.validity.clone(),
            appearance,
            path: self.path.clone(),
            rgba_data: Arc::new(Vec::new()),
            width: 0,
            height: 0,
            premultiplied: true,
        }
    }
}
