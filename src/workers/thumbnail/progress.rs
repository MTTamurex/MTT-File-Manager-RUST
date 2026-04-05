use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct BulkThumbnailProgress {
    pub root_name: String,
    pub current_file: String,
}

pub type SharedBulkThumbnailProgress = Arc<Mutex<Option<BulkThumbnailProgress>>>;

pub fn new_shared_bulk_thumbnail_progress() -> SharedBulkThumbnailProgress {
    Arc::new(Mutex::new(None))
}

pub fn begin_bulk_thumbnail_progress(progress: &SharedBulkThumbnailProgress, root: &Path) {
    if let Ok(mut guard) = progress.lock() {
        *guard = Some(BulkThumbnailProgress {
            root_name: display_name(root),
            current_file: String::new(),
        });
    }
}

pub fn set_bulk_thumbnail_current_file(progress: &SharedBulkThumbnailProgress, path: &Path) {
    if let Ok(mut guard) = progress.lock() {
        if let Some(state) = guard.as_mut() {
            state.current_file = display_name(path);
        }
    }
}

pub fn clear_bulk_thumbnail_progress(progress: &SharedBulkThumbnailProgress) {
    if let Ok(mut guard) = progress.lock() {
        *guard = None;
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}