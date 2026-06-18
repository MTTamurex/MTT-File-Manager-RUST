//! Thumbnail extraction pipeline
//!
//! Implements a hybrid extraction pipeline with image-only fast paths:
//!
//! 0. **Embedded EXIF JPEG Thumbnail** - Uses the low-resolution preview embedded in camera JPEGs when it satisfies the requested bucket
//! 1. **WIC Sized Decode** - Decodes large still images directly to the requested bucket
//! 2. **Image Crate** (Legacy Fast Path) - Uses `image` crate for common formats
//! 3. **WIC** (Robust Fallback) - Windows Imaging Component for CMYK/problematic images
//! 4. **Shell API** (Universal) - IThumbnailCache with WTS_EXTRACTDONOTCACHE
//! 5. **Media Foundation** (Fallback only) - Direct video frame extraction

pub mod orientation;
pub mod stage0_embedded_exif_thumbnail;
pub mod stage1_image_crate;
pub mod stage2_wic;
pub mod stage3_shell_api;
pub mod stage5_media_foundation;

use crate::domain::thumbnail::MAX_THUMBNAIL_SIDE;
use crate::infrastructure::diagnostic_logger::{
    diag_info, diag_warn, field_bool, field_duration_ms, field_label, field_u64,
};
use crate::infrastructure::io_priority::IOPriority;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows::file_type::is_video_extension;
use std::path::Path;
use std::time::Duration;

const SIZED_WIC_FAST_PATH_EXTENSIONS: &[&str] =
    &["jpg", "jpeg", "png", "bmp", "tiff", "tif", "webp"];
const DIAG_SLOW_STAGE_THRESHOLD: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub enum ThumbnailExtractionOutcome {
    Success((Vec<u8>, u32, u32)),
    UnsafeToRead(crate::infrastructure::windows::file_flags::FileReadSafety),
    Failed,
}

/// Hybrid thumbnail extraction pipeline
///
/// Attempts extraction in order of speed/reliability:
/// - Stages 1-2: Fast paths for images
/// - Stage 3: Universal Shell fallback without populating Explorer's cache
/// - Stage 5: Direct video frame extraction when Windows thumbnail providers fail
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
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !crate::infrastructure::windows::is_media_extension(ext) {
            log::trace!(
                "[Thumbnail] Skipping non-media file: {:?}",
                path.file_name()
            );
            return ThumbnailExtractionOutcome::Failed;
        }
    } else {
        return ThumbnailExtractionOutcome::Failed;
    }

    let pipeline_start = std::time::Instant::now();

    // Skip if file is pending deletion or no longer exists
    if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
        return ThumbnailExtractionOutcome::Failed;
    }

    // DEFENSE: Skip files that are still being downloaded or written to.
    let read_safety = crate::infrastructure::windows::file_flags::classify_file_read_safety(path);
    if read_safety != crate::infrastructure::windows::file_flags::FileReadSafety::Safe {
        return ThumbnailExtractionOutcome::UnsafeToRead(read_safety);
    }

    if let Some(max_side) = embedded_exif_thumbnail_target(path, image_target_max_side) {
        let t0 = std::time::Instant::now();
        if let Some(result) = stage0_embedded_exif_thumbnail::extract(path, priority, max_side) {
            log_extraction_perf(path, "stage0_exif", t0, &result);
            return ThumbnailExtractionOutcome::Success(result);
        }

        if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
            return ThumbnailExtractionOutcome::Failed;
        }
    }

    if let Some(max_side) = image_sized_fast_path_target(path, image_target_max_side) {
        let t0 = std::time::Instant::now();
        if let Some(result) = stage2_wic::extract_to_size_fast(path, Some(max_side)) {
            let result = orientation::apply_exif_orientation_to_thumbnail(path, priority, result);
            log_extraction_perf(path, "stage0_wic_sized", t0, &result);
            return ThumbnailExtractionOutcome::Success(result);
        }

        if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
            return ThumbnailExtractionOutcome::Failed;
        }
    }

    // Stage 1: image crate (Fast Path)
    {
        let t0 = std::time::Instant::now();
        if let Some(result) = stage1_image_crate::extract(path, priority, image_target_max_side) {
            let result = orientation::apply_exif_orientation_to_thumbnail(path, priority, result);
            log_extraction_perf(path, "stage1_image_crate", t0, &result);
            return ThumbnailExtractionOutcome::Success(result);
        }
    }

    if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
        return ThumbnailExtractionOutcome::Failed;
    }

    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    {
        let t0 = std::time::Instant::now();
        if let Some(result) = stage2_wic::extract_to_size(path, image_target_max_side) {
            let result = orientation::apply_exif_orientation_to_thumbnail(path, priority, result);
            log_extraction_perf(path, "stage2_wic", t0, &result);
            return ThumbnailExtractionOutcome::Success(result);
        }
    }

    if pending_deletions.contains_key(path) || !onedrive::fast_path_exists(path) {
        return ThumbnailExtractionOutcome::Failed;
    }

    // Shell API fallback (Universal/Video)
    {
        let t0 = std::time::Instant::now();
        match stage3_shell_api::extract_with_size(path, image_target_max_side) {
            Ok(result) => {
                log_extraction_perf(path, "stage3_shell", t0, &result);
                return ThumbnailExtractionOutcome::Success(result);
            }
            Err(e) => {
                log_extraction_stage_failure(path, "stage3_shell", t0);
                let err_str = e.to_string();
                if !err_str.contains("0x80070002") {
                    log::trace!(
                        "[Thumbnail] Stage 3 failed for {:?}: {}",
                        path.file_name(),
                        e
                    );
                }
            }
        }
    }

    // Media Foundation fallback only. It can return a technically valid but poor
    // thumbnail frame (black/wrong frame), so keep it behind Windows providers.
    {
        let t0 = std::time::Instant::now();
        if let Some(result) = stage5_media_foundation::extract(path) {
            log_extraction_perf(path, "stage5_mf", t0, &result);
            return ThumbnailExtractionOutcome::Success(result);
        }
        log_extraction_stage_failure(path, "stage5_mf", t0);
    }

    let total_ms = pipeline_start.elapsed().as_millis();
    log::warn!(
        "[Thumbnail] ALL STAGES FAILED for {:?} ({:.1}ms)",
        path.file_name(),
        total_ms as f64
    );
    diag_warn("thumbnail_extraction", "permanent_failure", &[]);
    ThumbnailExtractionOutcome::Failed
}

fn log_extraction_perf(
    path: &Path,
    stage: &'static str,
    start: std::time::Instant,
    result: &(Vec<u8>, u32, u32),
) {
    let elapsed = start.elapsed();
    let elapsed_ms = elapsed.as_millis();
    if elapsed_ms >= 25 {
        let (data, w, h) = result;
        log::info!(
            "[THUMB-PERF] {} {:?} {}x{} {}B {:.1}ms",
            stage,
            path.file_name(),
            w,
            h,
            data.len(),
            elapsed_ms as f64
        );
    }

    if elapsed >= DIAG_SLOW_STAGE_THRESHOLD {
        let (data, w, h) = result;
        diag_info(
            "thumbnail_extraction",
            "slow_stage_success",
            &[
                field_label("stage", stage),
                field_duration_ms("elapsed", elapsed),
                field_bool("video", is_video_path(path)),
                field_u64("width", *w as u64),
                field_u64("height", *h as u64),
                field_u64("bytes", data.len() as u64),
            ],
        );
    }
}

fn log_extraction_stage_failure(path: &Path, stage: &'static str, start: std::time::Instant) {
    let elapsed = start.elapsed();
    if elapsed < DIAG_SLOW_STAGE_THRESHOLD {
        return;
    }

    log::info!(
        "[THUMB-PERF] {} failed {:?} {:.1}ms",
        stage,
        path.file_name(),
        elapsed.as_millis() as f64
    );

    diag_info(
        "thumbnail_extraction",
        "slow_stage_failed",
        &[
            field_label("stage", stage),
            field_duration_ms("elapsed", elapsed),
            field_bool("video", is_video_path(path)),
        ],
    );
}

fn image_sized_fast_path_target(path: &Path, requested_max_side: Option<u32>) -> Option<u32> {
    let max_side = requested_max_side?.clamp(1, MAX_THUMBNAIL_SIDE);
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
    let max_side = requested_max_side?.clamp(1, MAX_THUMBNAIL_SIDE);
    let ext = path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
        Some(max_side)
    } else {
        None
    }
}

fn is_video_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| is_video_extension(&ext.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::{embedded_exif_thumbnail_target, image_sized_fast_path_target, is_video_path};
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
            Some(512)
        );
    }

    #[test]
    fn image_sized_fast_path_target_rejects_videos_and_missing_target() {
        assert_eq!(
            image_sized_fast_path_target(Path::new("clip.mp4"), Some(256)),
            None
        );
        assert_eq!(
            image_sized_fast_path_target(Path::new("anim.gif"), Some(256)),
            None
        );
        assert_eq!(
            image_sized_fast_path_target(Path::new("photo.jpg"), None),
            None
        );
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
        assert_eq!(
            embedded_exif_thumbnail_target(Path::new("photo.jpg"), None),
            None
        );
    }

    #[test]
    fn is_video_path_matches_video_extensions_only() {
        assert!(is_video_path(Path::new("clip.mp4")));
        assert!(is_video_path(Path::new("clip.MKV")));
        assert!(!is_video_path(Path::new("photo.jpg")));
    }
}
