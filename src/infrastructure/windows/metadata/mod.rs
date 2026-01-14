use std::path::Path;

pub mod property_keys;
pub mod utils;
pub mod image;
pub mod video;
pub mod video_sniffing;

pub use image::read_image_metadata;
pub use video::read_video_metadata;
pub use video_sniffing::sniff_video_codec;

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
}

/// Extracts metadata for common media types (images/videos).
/// Returns an empty struct when the file type is unsupported or metadata cannot be read.
pub fn extract_media_metadata(path: &Path) -> MediaMetadata {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    if image::is_image_extension(&ext) {
        return read_image_metadata(path).unwrap_or_default();
    }

    if video::is_video_extension(&ext) {
        return read_video_metadata(path).unwrap_or_default();
    }

    MediaMetadata::default()
}
