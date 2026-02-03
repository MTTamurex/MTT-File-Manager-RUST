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
pub fn generate_thumbnail_hybrid(
    path: &Path,
    priority: IOPriority,
) -> Option<(Vec<u8>, u32, u32)> {
    // Stage 1: image crate (Fast Path)
    if let Some(result) = stage1_image_crate::extract(path, priority) {
        return Some(result);
    }

    // Stage 2: WIC (Robust Fallback for JPEGs/CMYK)
    if let Some(result) = stage2_wic::extract(path) {
        return Some(result);
    }

    // Stage 3: Shell API (Universal/Video)
    match stage3_shell_api::extract(path) {
        Ok(result) => return Some(result),
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
    match stage4_force_extract::extract(path) {
        Ok(result) => return Some(result),
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
    stage5_media_foundation::extract(path)
}