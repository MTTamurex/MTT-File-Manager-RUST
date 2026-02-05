//! Thumbnail extraction pipeline
//!
//! Implements a 5-stage hybrid extraction pipeline:
//!
//! 1. **Image Crate** (Fast Path) - Uses `image` crate for common formats
//! 2. **WIC** (Robust Fallback) - Windows Imaging Component for CMYK/problematic images
//! 3. **Shell API** (Universal) - IShellItemImageFactory for most file types
//! 4. **Force Extraction** - IThumbnailCache with WTS_FORCEEXTRACTION flag
//! 5. **Media Foundation** (Nuclear Option) - Direct video frame extraction

pub mod stage1_image_crate;
pub mod stage2_wic;
pub mod stage3_shell_api;
pub mod stage4_force_extract;
pub mod stage5_media_foundation;

use crate::infrastructure::io_priority::IOPriority;
use std::path::Path;

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
    // DEFENSE IN DEPTH: Early exit for non-media files
    // This catches any requests that slipped through UI-level filtering (e.g., .exe, .dll)
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if !crate::infrastructure::windows::is_media_extension(ext) {
            eprintln!(
                "[Thumbnail] Skipping non-media file: {:?}",
                path.file_name()
            );
            return None;
        }
    } else {
        // No extension = skip
        return None;
    }

    eprintln!(
        "[Thumbnail] Starting extraction pipeline for: {:?}",
        path.file_name()
    );

    // Skip if file is pending deletion or no longer exists
    if pending_deletions.contains_key(path) || !path.exists() {
        return None;
    }

    // Stage 1: image crate (Fast Path)
    eprintln!("[Thumbnail] Trying Stage 1 (image crate)...");
    if let Some(result) = stage1_image_crate::extract(path, priority) {
        eprintln!("[Thumbnail] Stage 1 SUCCESS for: {:?}", path.file_name());
        return Some(result);
    }
    eprintln!("[Thumbnail] Stage 1 failed, trying Stage 2...");

    // Abort if file was deleted or marked for deletion during Stage 1
    if pending_deletions.contains_key(path) || !path.exists() {
        return None;
    }

    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    eprintln!("[Thumbnail] Trying Stage 2 (WIC)...");
    if let Some(result) = stage2_wic::extract(path) {
        eprintln!("[Thumbnail] Stage 2 SUCCESS for: {:?}", path.file_name());
        return Some(result);
    }
    eprintln!("[Thumbnail] Stage 2 failed, trying Stage 3...");

    // Abort if file was deleted or marked for deletion during Stage 2
    if pending_deletions.contains_key(path) || !path.exists() {
        return None;
    }

    // Stage 3: Shell API (Universal/Video)
    eprintln!("[Thumbnail] Trying Stage 3 (Shell API)...");
    match stage3_shell_api::extract(path) {
        Ok(result) => {
            eprintln!("[Thumbnail] Stage 3 SUCCESS for: {:?}", path.file_name());
            return Some(result);
        }
        Err(e) => {
            let err_str = e.to_string();
            // Don't log "File Not Found" errors as they are expected for recently deleted files
            if !err_str.contains("0x80070002") {
                eprintln!(
                    "[Thumbnail] Stage 3 failed for {:?}: {}",
                    path.file_name(),
                    e
                );
            }
        }
    }

    // Stage 4: IThumbnailCache with WTS_FORCEEXTRACTION (bypassa cache do Windows)
    // Útil quando o cache do Windows retornou um ícone em vez do thumbnail real
    // Single attempt - if fails, Stage 5 takes over
    eprintln!("[Thumbnail] Trying Stage 4 (Force Extract)...");
    match stage4_force_extract::extract(path) {
        Ok(result) => {
            eprintln!("[Thumbnail] Stage 4 SUCCESS for: {:?}", path.file_name());
            return Some(result);
        }
        Err(e) => {
            let err_str = e.to_string();
            // Don't log "File Not Found" errors as they are expected for recently deleted files
            if !err_str.contains("0x80070002") {
                eprintln!(
                    "[Thumbnail] Stage 4 (force) failed for {:?}: {}",
                    path.file_name(),
                    e
                );
            }
        }
    }

    // Stage 5: Media Foundation direct frame extraction (bypasses Windows thumbnail service)
    // This is the nuclear option - extracts a raw video frame when all else fails
    eprintln!("[Thumbnail] Trying Stage 5 (Media Foundation)...");
    let result = stage5_media_foundation::extract(path);
    if result.is_some() {
        eprintln!("[Thumbnail] Stage 5 SUCCESS for: {:?}", path.file_name());
    } else {
        eprintln!("[Thumbnail] ALL STAGES FAILED for: {:?}", path.file_name());
    }
    result
}
