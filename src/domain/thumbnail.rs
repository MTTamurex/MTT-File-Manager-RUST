use crate::infrastructure::io_priority::IOPriority;
use std::path::PathBuf;

/// Thumbnail data extracted from file
#[derive(Clone)]
pub struct ThumbnailData {
    pub path: PathBuf,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub generation: usize, // Tracks extraction validity
    pub priority: IOPriority,
    pub not_found: bool, // File no longer exists on disk
}
