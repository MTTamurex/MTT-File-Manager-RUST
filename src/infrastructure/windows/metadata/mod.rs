use std::path::Path;
use std::time::Duration;

pub mod audio;
pub mod audio_sniffing;
pub mod image;
pub mod property_keys;
pub mod utils;
pub mod video;
pub mod video_sniffing;

pub use audio::read_audio_metadata;
pub use audio_sniffing::{sniff_audio_codec, AudioCodec};
pub use image::read_image_metadata;
pub use video::read_video_metadata;
pub use video_sniffing::sniff_video_codec;

use crate::infrastructure::onedrive;

/// Maximum time allowed for the entire metadata extraction pipeline per file.
/// Catches any remaining blocking in Property Store, Media Foundation, codec sniffing, etc.
const METADATA_EXTRACTION_TIMEOUT_MS: u64 = 3000;

/// Generic media metadata used by the preview panel.
#[derive(Clone, Debug, Default)]
pub struct MediaMetadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Duration in 100-nanosecond ticks (same format as System.Media.Duration)
    pub duration_100ns: Option<u64>,
    /// Frames per second
    pub frame_rate: Option<f32>,
    /// Bitrate in bits per second (if available)
    pub bitrate: Option<u32>,
    /// File format label (PNG, JPEG, MP4, etc.)
    pub format: Option<String>,
    /// Color depth in bits per pixel (images only)
    pub color_depth: Option<u32>,

    // EXIF Data (Images)
    pub camera_maker: Option<String>,
    pub camera_model: Option<String>,
    pub f_stop: Option<String>,
    pub exposure_time: Option<String>,
    pub iso_speed: Option<u32>,
    pub focal_length: Option<String>,
    pub max_aperture: Option<String>,
    pub metering_mode: Option<String>,
    pub flash_mode: Option<String>,
    pub date_taken: Option<String>,
    pub subject: Option<String>,

    // Video Codec Info
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_bitrate: Option<u32>,
    pub audio_channels: Option<u32>,
    pub audio_sample_rate: Option<u32>,

    // Audio Tags (music files)
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_title: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
}

/// Extracts metadata for common media types (images/videos).
/// Returns an empty struct when the file type is unsupported or metadata cannot be read.
///
/// PERFORMANCE CRITICAL: For Cloud Files providers, checks local availability before reading
/// to prevent blocking on cloud-only files. Additionally wraps the entire extraction
/// in a timeout to catch any remaining blocking in COM/MF/sniffing calls.
pub fn extract_media_metadata(path: &Path) -> MediaMetadata {
    // CRITICAL FIX: Skip metadata extraction for cloud-only provider files
    // Reading metadata requires file I/O which can block indefinitely on cloud-only files
    if onedrive::is_cloud_sync_path(path) && !onedrive::is_locally_available(path) {
        log::debug!("[METADATA] Skipping cloud-only provider file: {:?}", path);
        return MediaMetadata::default();
    }

    // CRITICAL FIX: Skip files that are still being downloaded or written to.
    // Metadata extraction opens files via MFCreateSourceReaderFromURL,
    // SHGetPropertyStoreFromParsingName, File::open (codec sniffing), etc.
    // These APIs open the file WITHOUT FILE_SHARE_WRITE, causing sharing
    // violations that cancel active downloads (browsers, torrents, encoders).
    if crate::infrastructure::windows::file_flags::is_file_unsafe_to_read(path) {
        log::debug!(
            "[METADATA] Skipping file unsafe to read (download/write in progress): {:?}",
            path.file_name()
        );
        return MediaMetadata::default();
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    let is_image = image::is_image_extension(&ext);
    let is_video = video::is_video_extension(&ext);
    let is_audio = crate::infrastructure::windows::file_type::is_audio_extension(&ext);

    if !is_image && !is_video && !is_audio {
        return MediaMetadata::default();
    }

    // Apply timeout to video files (MPEG-TS etc. can block indefinitely in
    // Media Foundation), and to all Cloud Files paths (cloud filter driver can block).
    if is_video || onedrive::is_cloud_sync_path(path) {
        return extract_media_metadata_with_timeout(path, is_image, is_audio);
    }

    // Non-video, non-cloud: extract directly (no timeout overhead)
    extract_media_metadata_inner(path, is_image, is_audio)
}

/// Inner extraction logic (no timeout wrapper).
fn extract_media_metadata_inner(path: &Path, is_image: bool, is_audio: bool) -> MediaMetadata {
    if is_image {
        read_image_metadata(path).unwrap_or_default()
    } else if is_audio {
        read_audio_metadata(path).unwrap_or_default()
    } else {
        read_video_metadata(path).unwrap_or_default()
    }
}

/// Timeout-protected metadata extraction for video and OneDrive files.
///
/// CRITICAL FIX: Previously this function spawned a new `std::thread::spawn` per file
/// and abandoned the thread on timeout (dropping the `JoinHandle` without joining).
/// The abandoned thread continued running in kernel mode, blocked on COM/MF/cloud
/// filter driver I/O. Over prolonged use, these leaked threads accumulated and
/// congested the cloud filter driver, causing system-wide unresponsiveness.
///
/// Now uses the bounded I/O pool (`onedrive_io_pool().execute()`), which:
/// 1. Reuses a fixed set of worker threads (no unbounded thread creation)
/// 2. Has a capped overflow mechanism (max 24 temporary workers)
/// 3. Jobs that block in the pool don't create new kernel threads per call
fn extract_media_metadata_with_timeout(
    path: &Path,
    is_image: bool,
    is_audio: bool,
) -> MediaMetadata {
    let path_buf = path.to_path_buf();
    let path_for_log = path_buf.clone();
    let timeout = Duration::from_millis(METADATA_EXTRACTION_TIMEOUT_MS);

    let (tx, rx) = std::sync::mpsc::channel::<MediaMetadata>();

    let submitted = onedrive::onedrive_io_pool_execute(move || {
        let result = extract_media_metadata_inner(&path_buf, is_image, is_audio);
        let _ = tx.send(result);
    });

    if !submitted {
        log::warn!(
            "[METADATA] OneDrive I/O pool rejected job for {:?} — pool saturated",
            path_for_log
        );
        return MediaMetadata::default();
    }

    match rx.recv_timeout(timeout) {
        Ok(meta) => meta,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!(
                "[METADATA TIMEOUT] Extraction exceeded {}ms for {:?} — returning empty",
                METADATA_EXTRACTION_TIMEOUT_MS,
                path_for_log
            );
            MediaMetadata::default()
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            log::warn!(
                "[METADATA] Channel disconnected for {:?} — worker panicked",
                path_for_log
            );
            MediaMetadata::default()
        }
    }
}
