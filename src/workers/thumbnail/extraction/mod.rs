//! Thumbnail extraction pipeline
//!
//! Implements a hybrid extraction pipeline with image-only fast paths:
//!
//! 0. **Embedded EXIF JPEG Thumbnail** - Uses the low-resolution preview embedded in camera JPEGs when it satisfies the requested bucket
//! 1. **WIC Sized Decode** - Decodes large still images directly to the requested bucket
//! 2. **Image Crate** (Legacy Fast Path) - Uses `image` crate for common formats
//! 3. **WIC** (Robust Fallback) - Windows Imaging Component for CMYK/problematic images
//! 4. **Shell API** (Universal) - IShellItemImageFactory for most file types
//! 5. **Force Extraction** - IThumbnailCache with WTS_FORCEEXTRACTION flag
//! 6. **Media Foundation** (Nuclear Option) - Direct video frame extraction

pub mod stage0_embedded_exif_thumbnail;
pub mod stage1_image_crate;
pub mod stage2_wic;
pub mod stage3_shell_api;
pub mod stage4_force_extract;
pub mod stage5_media_foundation;

use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::onedrive;
use std::path::Path;

const SIZED_WIC_FAST_PATH_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "bmp", "tiff", "tif", "webp"];

#[derive(Debug)]
pub enum ThumbnailExtractionOutcome {
    Success((Vec<u8>, u32, u32)),
    UnsafeToRead(crate::infrastructure::windows::file_flags::FileReadSafety),
    Failed,
}

/// The 5-Step Hybrid Pipeline
///
/// Attempts extraction in order of speed/reliability:
/// - Stages 1-2: Fast paths for images
/// - Stage 3: Universal fallback
/// - Stage 4: Force bypass Windows cache
/// - Stage 5: Direct video frame extraction
///
/// `pending_deletions` is checked between stages to abort early if the file
/// was marked for deletion while extraction was in progress.
pub fn generate_thumbnail_hybrid(
    path: &Path,
    priority: IOPriority,
    pending_deletions: &dashmap::DashMap<std::path::PathBuf, ()>,
) -> Option<(Vec<u8>, u32, u32)> {
    match generate_thumbnail_hybrid_detailed_with_target(path, priority, pending_deletions, None) {
        ThumbnailExtractionOutcome::Success(data) => Some(data),
        ThumbnailExtractionOutcome::UnsafeToRead(_) | ThumbnailExtractionOutcome::Failed => None,
    }
}

pub fn generate_thumbnail_hybrid_detailed(
    path: &Path,
    priority: IOPriority,
    pending_deletions: &dashmap::DashMap<std::path::PathBuf, ()>,
) -> ThumbnailExtractionOutcome {
    generate_thumbnail_hybrid_detailed_with_target(path, priority, pending_deletions, None)
}

pub fn generate_thumbnail_hybrid_detailed_with_target(
    path: &Path,
    priority: IOPriority,
    pending_deletions: &dashmap::DashMap<std::path::PathBuf, ()>,
    image_target_max_side: Option<u32>,
) -> ThumbnailExtractionOutcome {
    // DEFENSE IN DEPTH: Early exit for non-media files
    // This catches any requests that slipped through UI-level filtering (e.g., .exe, .dll)
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !crate::infrastructure::windows::is_media_extension(ext) {
            log::trace!(
                "[Thumbnail] Skipping non-media file: {:?}",
                path.file_name()
            );
            return ThumbnailExtractionOutcome::Failed;
        }
    } else {
        // No extension = skip
        return ThumbnailExtractionOutcome::Failed;
    }

    log::trace!(
        "[Thumbnail] Starting extraction pipeline for: {:?}",
        path.file_name()
    );

    // Skip if file is pending deletion or no longer exists
    // Use fast_path_exists (GetFileAttributesW) instead of path.exists() (CreateFileW)
    // to avoid triggering OneDrive downloads and reduce HDD seek overhead
    if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
        return ThumbnailExtractionOutcome::Failed;
    }

    // DEFENSE: Skip files that are still being downloaded or written to.
    // Reading them can interrupt active downloads (sharing violation) or
    // produce corrupt/partial thumbnails from incomplete data.
    let read_safety = crate::infrastructure::windows::file_flags::classify_file_read_safety(path);
    if read_safety != crate::infrastructure::windows::file_flags::FileReadSafety::Safe {
        return ThumbnailExtractionOutcome::UnsafeToRead(read_safety);
    }

    if let Some(max_side) = embedded_exif_thumbnail_target(path, image_target_max_side) {
        log::trace!(
            "[Thumbnail] Trying Stage EXIF (embedded JPEG thumbnail, {}px)...",
            max_side
        );
        if let Some(result) = stage0_embedded_exif_thumbnail::extract(path, priority, max_side) {
            log::trace!("[Thumbnail] Stage EXIF SUCCESS for: {:?}", path.file_name());
            return ThumbnailExtractionOutcome::Success(result);
        }
        log::trace!("[Thumbnail] Stage EXIF unavailable or too small, trying Stage 0...");

        if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
            return ThumbnailExtractionOutcome::Failed;
        }
    }

    if let Some(max_side) = image_sized_fast_path_target(path, image_target_max_side) {
        log::trace!(
            "[Thumbnail] Trying Stage 0 (WIC sized image fast path, {}px)...",
            max_side
        );
        if let Some(result) = stage2_wic::extract_to_size(path, Some(max_side)) {
            log::trace!("[Thumbnail] Stage 0 SUCCESS for: {:?}", path.file_name());
            return ThumbnailExtractionOutcome::Success(result);
        }
        log::trace!("[Thumbnail] Stage 0 failed, trying Stage 1...");

        if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
            return ThumbnailExtractionOutcome::Failed;
        }
    }

    // Stage 1: image crate (Fast Path)
    log::trace!("[Thumbnail] Trying Stage 1 (image crate)...");
    if let Some(result) = stage1_image_crate::extract(path, priority) {
        log::trace!("[Thumbnail] Stage 1 SUCCESS for: {:?}", path.file_name());
        return ThumbnailExtractionOutcome::Success(result);
    }
    log::trace!("[Thumbnail] Stage 1 failed, trying Stage 2...");

    // Abort if file was deleted or marked for deletion during Stage 1
    if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
        return ThumbnailExtractionOutcome::Failed;
    }

    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    log::trace!("[Thumbnail] Trying Stage 2 (WIC)...");
    if let Some(result) = stage2_wic::extract(path) {
        log::trace!("[Thumbnail] Stage 2 SUCCESS for: {:?}", path.file_name());
        return ThumbnailExtractionOutcome::Success(result);
    }
    log::trace!("[Thumbnail] Stage 2 failed, trying Stage 3...");

    // Abort if file was deleted or marked for deletion during Stage 2
    if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
        return ThumbnailExtractionOutcome::Failed;
    }

    // Stage 3: Shell API (Universal/Video)
    log::trace!("[Thumbnail] Trying Stage 3 (Shell API)...");
    match stage3_shell_api::extract(path) {
        Ok(result) => {
            log::trace!("[Thumbnail] Stage 3 SUCCESS for: {:?}", path.file_name());
            return ThumbnailExtractionOutcome::Success(result);
        }
        Err(e) => {
            let err_str = e.to_string();
            // Don't log "File Not Found" errors as they are expected for recently deleted files
            if !err_str.contains("0x80070002") {
                log::trace!(
                    "[Thumbnail] Stage 3 failed for {:?}: {}",
                    path.file_name(),
                    e
                );
            }
        }
    }

    // Stage 4: IThumbnailCache with WTS_FORCEEXTRACTION (bypasses Windows cache)
    // Useful when Windows cache returned an icon instead of the actual thumbnail
    // Single attempt - if fails, Stage 5 takes over
    log::trace!("[Thumbnail] Trying Stage 4 (Force Extract)...");
    match stage4_force_extract::extract(path) {
        Ok(result) => {
            log::trace!("[Thumbnail] Stage 4 SUCCESS for: {:?}", path.file_name());
            return ThumbnailExtractionOutcome::Success(result);
        }
        Err(e) => {
            let err_str = e.to_string();
            // Don't log "File Not Found" errors as they are expected for recently deleted files
            if !err_str.contains("0x80070002") {
                log::trace!(
                    "[Thumbnail] Stage 4 (force) failed for {:?}: {}",
                    path.file_name(),
                    e
                );
            }
        }
    }

    // Stage 5: Media Foundation direct frame extraction (bypasses Windows thumbnail service)
    // This is the nuclear option - extracts a raw video frame when all else fails
    log::trace!("[Thumbnail] Trying Stage 5 (Media Foundation)...");
    if let Some(result) = stage5_media_foundation::extract(path) {
        log::trace!("[Thumbnail] Stage 5 SUCCESS for: {:?}", path.file_name());
        return ThumbnailExtractionOutcome::Success(result);
    } else {
        log::warn!("[Thumbnail] ALL STAGES FAILED for: {:?}", path.file_name());
    }
    ThumbnailExtractionOutcome::Failed
}

fn image_sized_fast_path_target(path: &Path, requested_max_side: Option<u32>) -> Option<u32> {
    let max_side = requested_max_side?.max(1);
    let ext = path.extension()?.to_str()?;
    if SIZED_WIC_FAST_PATH_EXTENSIONS
        .iter()
        .any(|candidate| ext.eq_ignore_ascii_case(candidate))
    {
        Some(max_side)
    } else {
        None
    }
}

fn embedded_exif_thumbnail_target(path: &Path, requested_max_side: Option<u32>) -> Option<u32> {
    let max_side = requested_max_side?.max(1);
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        Some(max_side)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{embedded_exif_thumbnail_target, image_sized_fast_path_target};
    use std::path::Path;

    #[test]
    fn image_sized_fast_path_target_matches_supported_still_images() {
        assert_eq!(
            image_sized_fast_path_target(Path::new("photo.JPG"), Some(256)),
            Some(256)
        );
        assert_eq!(
            image_sized_fast_path_target(Path::new("poster.png"), Some(512)),
            Some(512)
        );
        assert_eq!(
            image_sized_fast_path_target(Path::new("cover.webp"), Some(1024)),
            Some(1024)
        );
    }

    #[test]
    fn image_sized_fast_path_target_rejects_videos_and_missing_target() {
        assert_eq!(
            image_sized_fast_path_target(Path::new("clip.mp4"), Some(256)),
            None
        );
        assert_eq!(image_sized_fast_path_target(Path::new("anim.gif"), Some(256)), None);
        assert_eq!(image_sized_fast_path_target(Path::new("photo.jpg"), None), None);
    }

    #[test]
    fn embedded_exif_thumbnail_target_accepts_only_jpeg_requests() {
        assert_eq!(
            embedded_exif_thumbnail_target(Path::new("photo.jpeg"), Some(256)),
            Some(256)
        );
        assert_eq!(
            embedded_exif_thumbnail_target(Path::new("photo.jpg"), Some(512)),
            Some(512)
        );
        assert_eq!(
            embedded_exif_thumbnail_target(Path::new("photo.png"), Some(256)),
            None
        );
        assert_eq!(embedded_exif_thumbnail_target(Path::new("photo.jpg"), None), None);
    }
}
