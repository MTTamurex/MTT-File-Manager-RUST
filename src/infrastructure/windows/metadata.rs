//! Media metadata extraction via Windows Property Store, MediaFoundation, and image headers
//! Follows .cursorrules: single responsibility
//!
//! Strategy: Property Store (fast) -> MediaFoundation fallback (reliable)

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use super::media_foundation::extract_video_metadata_mf;
use exif::{In, Reader as ExifReader, Tag};
use image::ImageReader;
use windows::{
    core::{GUID, PCWSTR},
    Win32::Foundation::RPC_E_CHANGED_MODE,
    Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreFromParsingName, GETPROPERTYSTOREFLAGS, GPS_OPENSLOWITEM,
        GPS_READWRITE,
    },
};

// Manual property key definitions (from Propkey.h)
// These are not exposed by windows-rs, so we define them manually
#[repr(C)]
#[derive(Clone, Copy)]
struct PROPERTYKEY {
    fmtid: GUID,
    pid: u32,
}

// System.Media.Duration (64440490-4C8B-11D1-8B70-080036B11A03, 3)
const PKEY_MEDIA_DURATION: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 3,
};

// System.Video.FrameWidth (64440491-4C8B-11D1-8B70-080036B11A03, 3)
const PKEY_VIDEO_FRAMEWIDTH: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 3,
};

// System.Video.FrameHeight (64440491-4C8B-11D1-8B70-080036B11A03, 4)
const PKEY_VIDEO_FRAMEHEIGHT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 4,
};

// System.Video.FrameRate (64440491-4C8B-11D1-8B70-080036B11A03, 6)
const PKEY_VIDEO_FRAMERATE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 6,
};

// System.Image.CameraModel (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 272)
const PKEY_IMAGE_CAMERAMODEL: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 272,
};

// System.Image.CameraMaker (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 271)
const PKEY_IMAGE_CAMERAMAKER: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 271,
};

// System.Photo.FNumber (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 33437)
const PKEY_IMAGE_FNUMBER: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 33437,
};

// System.Photo.ExposureTime (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 33434)
const PKEY_IMAGE_EXPOSURETIME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 33434,
};

// System.Photo.ISOSpeed (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 34855)
const PKEY_IMAGE_ISOSPEED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 34855,
};

// System.Photo.FocalLength (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 37386)
const PKEY_IMAGE_FOCALLENGTH: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 37386,
};

// System.Photo.MaxAperture (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 37381)
const PKEY_IMAGE_MAXAPERTURE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 37381,
};

// System.Photo.MeteringMode (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 37383)
const PKEY_IMAGE_METERINGMODE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 37383,
};

// System.Photo.Flash (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 37385)
const PKEY_IMAGE_FLASH: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 37385,
};

// System.Photo.DateTaken (14B81DA1-0135-4D31-96D9-6CBFC9671A99, 36867)
const PKEY_IMAGE_DATETAKEN: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x14B81DA1_0135_4D31_96D9_6CBFC9671A99),
    pid: 36867,
};

// System.Subject (F29F85E0-4FF9-1068-AB91-08002B27B3D9, 3)
const PKEY_IMAGE_SUBJECT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xF29F85E0_4FF9_1068_AB91_08002B27B3D9),
    pid: 3,
};

// System.Media.SubTitle (56A3372E-CE9C-11D2-9F0E-006097C686F6, 38) - Used for "Video tracks" in Explorer
const PKEY_MEDIA_SUBTITLE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x56A3372E_CE9C_11D2_9F0E_006097C686F6),
    pid: 38,
};

// System.Media.EncodingSettings (64440490-4C8B-11D1-8B70-080036B11A03, 10)
const PKEY_MEDIA_ENCODINGSETTINGS: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 10,
};

// System.Audio.EncodingBitrate (64440490-4C8B-11D1-8B70-080036B11A03, 4)
const PKEY_AUDIO_ENCODINGBITRATE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 4,
};

// System.Audio.ChannelCount (64440490-4C8B-11D1-8B70-080036B11A03, 7)
const PKEY_AUDIO_CHANNELCOUNT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 7,
};

// System.Video.EncodingBitrate (64440491-4C8B-11D1-8B70-080036B11A03, 8)
const PKEY_VIDEO_ENCODINGBITRATE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 8,
};

// System.Video.FourCC (64440491-4C8B-11D1-8B70-080036B11A03, 44)
const PKEY_VIDEO_FOURCC: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 44,
};

// System.Media.ContentType (64440492-4C8B-11D1-8B70-080036B11A03, 1)
const PKEY_MEDIA_CONTENTTYPE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440492_4C8B_11D1_8B70_080036B11A03),
    pid: 1,
};

// System.Video.Compression (64440491-4C8B-11D1-8B70-080036B11A03, 10)
const PKEY_VIDEO_COMPRESSION: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 10,
};

// System.Video.StreamName (64440491-4C8B-11D1-8B70-080036B11A03, 2) - Used by K-Lite/Icaros
const PKEY_VIDEO_STREAMNAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440491_4C8B_11D1_8B70_080036B11A03),
    pid: 2,
};

// System.Audio.Format (64440490-4C8B-11D1-8B70-080036B11A03, 2)
const PKEY_AUDIO_FORMAT: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 2,
};

// System.Audio.StreamName (64440490-4C8B-11D1-8B70-080036B11A03, 9)
const PKEY_AUDIO_STREAMNAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 9,
};

// System.Audio.Compression (64440490-4C8B-11D1-8B70-080036B11A03, 10) - K-Lite/Icaros populates this!
const PKEY_AUDIO_COMPRESSION: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x64440490_4C8B_11D1_8B70_080036B11A03),
    pid: 10,
};

// PROPVARIANT type tags (from WTypes.h)
const VT_UI4: u16 = 19;
const VT_UI8: u16 = 21;
const VT_I4: u16 = 3;
const VT_I8: u16 = 20;
const VT_R8: u16 = 5; // Double (64-bit float)
const VT_UI2: u16 = 18; // Unsigned 16-bit int
const VT_I2: u16 = 2; // Signed 16-bit int

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

    if is_image_extension(&ext) {
        return read_image_metadata(path).unwrap_or_default();
    }

    if is_video_extension(&ext) {
        return read_video_metadata(path).unwrap_or_default();
    }

    MediaMetadata::default()
}

fn is_image_extension(ext: &str) -> bool {
    // Use Windows Perceived Type API for dynamic detection
    super::file_type::is_image_extension(ext)
}

fn is_video_extension(ext: &str) -> bool {
    // Use Windows Perceived Type API for dynamic detection
    // This includes OGM, MKV, WebM, and any format K-Lite/Icaros registers
    super::file_type::is_video_extension(ext)
}

fn read_image_exif_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    eprintln!("[EXIF DEBUG] Reading EXIF for: {:?}", path.file_name());

    // Try direct EXIF reading first (more reliable than Property Store for JPEG)
    let mut camera_maker = None;
    let mut camera_model = None;
    let mut f_stop = None;
    let mut exposure_time = None;
    let mut iso_speed = None;
    let mut focal_length = None;
    let mut max_aperture = None;
    let mut metering_mode = None;
    let mut flash_mode = None;
    let mut date_taken = None;
    let mut subject = None;

    if let Ok(file) = std::fs::File::open(path) {
        let mut bufreader = std::io::BufReader::new(&file);
        if let Ok(exifreader) = ExifReader::new().read_from_container(&mut bufreader) {
            eprintln!("  [EXIF] Successfully parsed EXIF data");

            // Camera Make
            if let Some(field) = exifreader.get_field(Tag::Make, In::PRIMARY) {
                camera_maker = Some(field.display_value().to_string());
            }

            // Camera Model
            if let Some(field) = exifreader.get_field(Tag::Model, In::PRIMARY) {
                camera_model = Some(field.display_value().to_string());
            }

            // F-Number
            if let Some(field) = exifreader.get_field(Tag::FNumber, In::PRIMARY) {
                f_stop = Some(format!("f/{}", field.display_value()));
            }

            // Exposure Time
            if let Some(field) = exifreader.get_field(Tag::ExposureTime, In::PRIMARY) {
                exposure_time = Some(format!("{} sec.", field.display_value()));
            }

            // ISO Speed
            if let Some(field) = exifreader.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
                if let exif::Value::Short(ref v) = field.value {
                    if !v.is_empty() {
                        iso_speed = Some(v[0] as u32);
                    }
                }
            }

            // Focal Length
            if let Some(field) = exifreader.get_field(Tag::FocalLength, In::PRIMARY) {
                focal_length = Some(format!("{} mm", field.display_value()));
            }

            // Max Aperture
            if let Some(field) = exifreader.get_field(Tag::MaxApertureValue, In::PRIMARY) {
                max_aperture = Some(field.display_value().to_string());
            }

            // Metering Mode
            if let Some(field) = exifreader.get_field(Tag::MeteringMode, In::PRIMARY) {
                metering_mode = Some(field.display_value().to_string());
            }

            // Flash
            if let Some(field) = exifreader.get_field(Tag::Flash, In::PRIMARY) {
                flash_mode = Some(field.display_value().to_string());
            }

            // Date Taken
            if let Some(field) = exifreader.get_field(Tag::DateTime, In::PRIMARY) {
                date_taken = Some(field.display_value().to_string());
            }

            // Subject/Description
            if let Some(field) = exifreader.get_field(Tag::ImageDescription, In::PRIMARY) {
                subject = Some(field.display_value().to_string());
            }
        } else {
            eprintln!("  [EXIF] Failed to parse EXIF data, trying Property Store fallback");

            // Fallback to Property Store if direct EXIF reading fails
            let _com_guard = ComGuard::new();
            if let Ok(store) = unsafe { open_property_store(path) } {
                camera_maker = unsafe { read_string(&store, &PKEY_IMAGE_CAMERAMAKER) };
                camera_model = unsafe { read_string(&store, &PKEY_IMAGE_CAMERAMODEL) };
                f_stop = unsafe { read_f_number(&store, &PKEY_IMAGE_FNUMBER) };
                exposure_time = unsafe { read_exposure_time(&store, &PKEY_IMAGE_EXPOSURETIME) };
                iso_speed = unsafe { read_u32(&store, &PKEY_IMAGE_ISOSPEED) };
                focal_length = unsafe { read_focal_length(&store, &PKEY_IMAGE_FOCALLENGTH) };
                max_aperture = unsafe { read_aperture(&store, &PKEY_IMAGE_MAXAPERTURE) };
                metering_mode = unsafe { read_metering_mode(&store, &PKEY_IMAGE_METERINGMODE) };
                flash_mode = unsafe { read_flash_mode(&store, &PKEY_IMAGE_FLASH) };
                date_taken = unsafe { read_string(&store, &PKEY_IMAGE_DATETAKEN) };
                subject = unsafe { read_string(&store, &PKEY_IMAGE_SUBJECT) };
            }
        }
    }

    eprintln!("  camera_maker: {:?}", camera_maker);
    eprintln!("  camera_model: {:?}", camera_model);
    eprintln!("  f_stop: {:?}", f_stop);
    eprintln!("  exposure_time: {:?}", exposure_time);
    eprintln!("  iso_speed: {:?}", iso_speed);
    eprintln!("  focal_length: {:?}", focal_length);
    eprintln!("  flash_mode: {:?}", flash_mode);

    Ok(MediaMetadata {
        width: None,
        height: None,
        duration_100ns: None,
        frame_rate: None,
        bitrate: None,
        format: None,
        color_depth: None,
        camera_maker,
        camera_model,
        f_stop,
        exposure_time,
        iso_speed,
        focal_length,
        max_aperture,
        metering_mode,
        flash_mode,
        date_taken,
        subject,
        video_codec: None,
        audio_codec: None,
        audio_bitrate: None,
        audio_channels: None,
    })
}

fn read_image_metadata(path: &Path) -> Result<MediaMetadata, image::ImageError> {
    // Uses image crate headers only; does not decode the full image.
    let reader = ImageReader::open(path)?;
    let reader = reader.with_guessed_format()?;
    let format_label = reader.format().map(|f| format!("{:?}", f).to_uppercase());
    let (width, height) = reader.into_dimensions()?;

    // Try to also read EXIF data from property store if available
    let exif_metadata = read_image_exif_metadata(path).unwrap_or_default();

    Ok(MediaMetadata {
        width: Some(width),
        height: Some(height),
        duration_100ns: None,
        frame_rate: None,
        bitrate: None,
        format: format_label,
        color_depth: None,
        camera_maker: exif_metadata.camera_maker,
        camera_model: exif_metadata.camera_model,
        f_stop: exif_metadata.f_stop,
        exposure_time: exif_metadata.exposure_time,
        iso_speed: exif_metadata.iso_speed,
        focal_length: exif_metadata.focal_length,
        max_aperture: exif_metadata.max_aperture,
        metering_mode: exif_metadata.metering_mode,
        flash_mode: exif_metadata.flash_mode,
        date_taken: exif_metadata.date_taken,
        subject: exif_metadata.subject,
        video_codec: None,
        audio_codec: None,
        audio_bitrate: None,
        audio_channels: None,
    })
}

fn read_video_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    // Try Property Store first (fast, uses Windows cache)
    let ps_result = read_video_via_property_store(path);

    // Split result to allow merging with MediaFoundation when codecs/frame rate are missing
    let (mut ps_meta_opt, ps_err_opt) = match ps_result {
        Ok(meta) => (Some(meta), None),
        Err(e) => (None, Some(e)),
    };

    // Decide if we need MediaFoundation: missing core fields OR missing codecs/frame rate
    let need_mf = match &ps_meta_opt {
        Some(meta) => {
            meta.width.is_none()
                || meta.height.is_none()
                || meta.duration_100ns.is_none()
                || meta.video_codec.is_none()
                || meta.audio_codec.is_none()
                || meta.frame_rate.is_none()
        }
        None => true,
    };

    if need_mf {
        if let Some(mf_meta) = extract_video_metadata_mf(path) {
            let base = ps_meta_opt.take().unwrap_or_default();
            return Ok(merge_video_metadata(base, mf_meta, path));
        }
    }

    if let Some(meta) = ps_meta_opt {
        Ok(meta)
    } else {
        Err(ps_err_opt.unwrap())
    }
}

/// Merge Property Store metadata with MediaFoundation metadata
fn merge_video_metadata(
    ps: MediaMetadata,
    mf: super::media_foundation::VideoMetadataMF,
    path: &Path,
) -> MediaMetadata {
    let frame_rate = ps
        .frame_rate
        .or_else(|| match (mf.frame_rate_num, mf.frame_rate_den) {
            (Some(num), Some(den)) if den > 0 => Some(num as f32 / den as f32),
            _ => None,
        });

    let video_codec = ps
        .video_codec
        .or_else(|| mf.video_codec_guid.clone())
        .map(|s| sanitize_codec_string(&s));

    let audio_codec = ps
        .audio_codec
        .or_else(|| mf.audio_codec_guid.clone())
        .map(|s| sanitize_codec_string(&s));

    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.to_uppercase());

    // Calculate bitrate from file size if not available
    let duration_100ns = ps.duration_100ns.or(mf.duration_100ns);
    let bitrate = ps.bitrate.or(mf.video_bitrate).or_else(|| {
        if let Some(duration) = duration_100ns {
            if duration > 0 {
                if let Ok(file_meta) = std::fs::metadata(path) {
                    let size_bytes = file_meta.len();
                    let duration_seconds = duration as f64 / 10_000_000.0;
                    return Some((size_bytes as f64 * 8.0 / duration_seconds) as u32);
                }
            }
        }
        None
    });

    MediaMetadata {
        width: ps.width.or(mf.width),
        height: ps.height.or(mf.height),
        duration_100ns,
        frame_rate,
        bitrate,
        format,
        color_depth: None,
        camera_maker: None,
        camera_model: None,
        f_stop: None,
        exposure_time: None,
        iso_speed: None,
        focal_length: None,
        max_aperture: None,
        metering_mode: None,
        flash_mode: None,
        date_taken: None,
        subject: None,
        video_codec,
        audio_codec,
        audio_bitrate: ps.audio_bitrate.or(mf.audio_bitrate),
        audio_channels: ps.audio_channels.or(mf.audio_channels),
    }
}

const PKEY_VIDEO_TRACKS: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xEC59938F_E25E_4592_82E3_8013018EDB74), // OGM Video Tracks
    pid: 3,
};

const PKEY_AUDIO_TRACKS: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x6F6B78A7_F4A2_4F9F_86B1_C6AAA6DEC9A5), // OGM Audio Tracks
    pid: 4,
};

/// Read video metadata using Windows Property Store (fast path)
fn read_video_via_property_store(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    let _com_guard = ComGuard::new();
    let store = unsafe { open_property_store(path)? };

    let width = unsafe { read_u32(&store, &PKEY_VIDEO_FRAMEWIDTH) };
    let height = unsafe { read_u32(&store, &PKEY_VIDEO_FRAMEHEIGHT) };
    let duration_100ns = unsafe { read_u64(&store, &PKEY_MEDIA_DURATION) };
    let frame_rate = unsafe { read_u32(&store, &PKEY_VIDEO_FRAMERATE) }.and_then(|raw| {
        if raw == 0 {
            None
        } else {
            Some(raw as f32 / 1_000.0)
        }
    });

    // PRIORITY FALLBACK SYSTEM for Video Codec (K-Lite/Icaros friendly)
    // Priority 1: FourCC (raw technical identifier - most accurate)
    let fourcc = unsafe { read_fourcc(&store, &PKEY_VIDEO_FOURCC) };
    let stream_name = unsafe { read_string(&store, &PKEY_VIDEO_STREAMNAME) };
    let subtitle = unsafe { read_string(&store, &PKEY_MEDIA_SUBTITLE) };
    let encoding_settings = unsafe { read_string(&store, &PKEY_MEDIA_ENCODINGSETTINGS) };
    let content_type = unsafe { read_string(&store, &PKEY_MEDIA_CONTENTTYPE) };
    let compression = unsafe { read_string(&store, &PKEY_VIDEO_COMPRESSION) };

    let ogm_video = unsafe { read_string(&store, &PKEY_VIDEO_TRACKS) };
    let ogm_audio = unsafe { read_string(&store, &PKEY_AUDIO_TRACKS) };

    let video_codec = fourcc
        .or_else(|| stream_name.clone())
        .or_else(|| ogm_video.clone())
        .or_else(|| subtitle.clone())
        .or_else(|| encoding_settings.clone())
        .or_else(|| content_type.clone())
        .or_else(|| {
            let comp = compression.clone()?;
            // Filter out container names and generic labels
            let file_ext = path.extension()?.to_str()?.to_uppercase();
            let compression_upper = comp.to_uppercase();
            if compression_upper == file_ext || compression_upper == "VIDEO" {
                None // Skip container names
            } else {
                Some(comp)
            }
        });

    // PRIORITY FALLBACK SYSTEM for Audio Codec
    let audio_compression = unsafe { read_string(&store, &PKEY_AUDIO_COMPRESSION) };
    let audio_stream_name = unsafe { read_string(&store, &PKEY_AUDIO_STREAMNAME) };
    let audio_format = unsafe { read_string(&store, &PKEY_AUDIO_FORMAT) };

    let audio_codec = audio_compression
        .or_else(|| audio_stream_name.clone())
        .or_else(|| ogm_audio.clone())
        .or_else(|| audio_format.clone())
        .map(|s| sanitize_codec_string(&s));

    // Audio metadata
    let audio_bitrate = unsafe { read_u32(&store, &PKEY_AUDIO_ENCODINGBITRATE) };
    let audio_channels = unsafe { read_u32(&store, &PKEY_AUDIO_CHANNELCOUNT) };

    // Video bitrate
    let video_bitrate = unsafe { read_u32(&store, &PKEY_VIDEO_ENCODINGBITRATE) };

    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.to_uppercase());

    // Intelligent fallback: extract codec hints from filename (common naming patterns)
    let video_codec_final = video_codec.or_else(|| {
        let filename = path.file_name()?.to_str()?;
        let filename_lower = filename.to_lowercase();

        // Check for common codec indicators in filename
        if filename_lower.contains("x264")
            || filename_lower.contains("h264")
            || filename_lower.contains("avc")
        {
            return Some("H.264/AVC".to_string());
        }
        if filename_lower.contains("x265")
            || filename_lower.contains("h265")
            || filename_lower.contains("hevc")
        {
            return Some("H.265/HEVC".to_string());
        }
        if filename_lower.contains("av1") {
            return Some("AV1".to_string());
        }
        if filename_lower.contains("vp9") {
            return Some("VP9".to_string());
        }
        if filename_lower.contains("vp8") {
            return Some("VP8".to_string());
        }
        if filename_lower.contains("divx") || filename_lower.contains("dx50") {
            return Some("DivX".to_string());
        }
        if filename_lower.contains("xvid") {
            return Some("XviD".to_string());
        }

        // Don't return container name as codec - better to show nothing
        None
    });

    // Sanitize video codec string (convert GUIDs to names, filter container names)
    let video_codec_sanitized = video_codec_final
        .map(|s| sanitize_codec_string(&s))
        .filter(|s| !s.is_empty() && !is_container_name(s, path));

    // Calculate total bitrate from file size if Property Store doesn't have it
    let bitrate = video_bitrate.or_else(|| {
        if let Some(duration) = duration_100ns {
            if duration > 0 {
                if let Ok(metadata) = std::fs::metadata(path) {
                    let size_bytes = metadata.len();
                    let duration_seconds = duration as f64 / 10_000_000.0;
                    let bitrate_bps = (size_bytes as f64 * 8.0) / duration_seconds;
                    return Some(bitrate_bps as u32);
                }
            }
        }
        None
    });

    Ok(MediaMetadata {
        width,
        height,
        duration_100ns,
        frame_rate,
        bitrate,
        format,
        color_depth: None,
        camera_maker: None,
        camera_model: None,
        f_stop: None,
        exposure_time: None,
        iso_speed: None,
        focal_length: None,
        max_aperture: None,
        metering_mode: None,
        flash_mode: None,
        date_taken: None,
        subject: None,
        video_codec: video_codec_sanitized,
        audio_codec,
        audio_bitrate,
        audio_channels,
    })
}

/// Convert GUID strings and technical codec identifiers to friendly names
fn sanitize_codec_string(s: &str) -> String {
    let s = s.trim();

    // Quick GUID substring checks for common audio codecs that sometimes leak as GUID strings
    let upper = s.to_ascii_uppercase();
    if upper.contains("0000704F") {
        // {0000704F-0000-0010-8000-00AA00389B71} → Opus
        return "Opus".to_string();
    }
    if upper.contains("00001FCA") {
        // {00001FCA-0000-0010-8000-00AA00389B71} → AV1 (rare audio GUID form)
        return "AV1".to_string();
    }
    if upper.contains("8D2FD10B") {
        // {8D2FD10B-5841-4A6B-8905-588FEC1ADED9} → Vorbis (MEDIASUBTYPE_Vorbis2)
        return "Vorbis".to_string();
    }
    if upper.contains("E06D802C") {
        // {E06D802C-DB46-11CF-B4D1-00805F6CBBEA} → Dolby AC-3 (MEDIASUBTYPE_DOLBY_AC3)
        return "Dolby AC-3".to_string();
    }

    // Check if it's a GUID string like "{00001610-0000-0010-8000-00AA00389B71}"
    if s.starts_with('{') && s.contains('-') {
        // Extract the first segment (data1)
        if let Some(data1_str) = s.strip_prefix('{').and_then(|s| s.split('-').next()) {
            if let Ok(data1) = u32::from_str_radix(data1_str, 16) {
                // Common audio format tags
                return match data1 {
                    0x0001 => "PCM".to_string(),
                    0x0003 => "IEEE Float".to_string(),
                    0x0055 => "MP3".to_string(),
                    0x00FF => "AAC".to_string(),
                    0x004F70 => "Opus".to_string(), // GUIDs that encode Opus as 0x0000704F
                    0x0000704F => "Opus".to_string(), // Another Opus variant seen in Property Store
                    0x0160 => "WMA v1".to_string(),
                    0x0161 => "WMA v2".to_string(),
                    0x0162 => "WMA Pro".to_string(),
                    0x0163 => "WMA Lossless".to_string(),
                    0x1610 => "AAC-LC".to_string(),
                    0x1612 => "AAC-HE".to_string(),
                    0xA106 => "AAC (ADTS)".to_string(),
                    0x2000 => "AC-3".to_string(),
                    0x2001 => "DTS".to_string(),

                    // FourCC-based codecs (higher numbers)
                    0x6134706D => "AAC".to_string(),        // 'mp4a'
                    0x7375704F => "Opus".to_string(),       // 'Opus'
                    0x43414C46 => "FLAC".to_string(),       // 'FLAC'
                    0x30395056 => "VP9".to_string(),        // 'VP90'
                    0x30385056 => "VP8".to_string(),        // 'VP80'
                    0x31305641 => "AV1".to_string(),        // 'AV01'
                    0x31435641 => "H.264/AVC".to_string(),  // 'AVC1'
                    0x43564548 => "H.265/HEVC".to_string(), // 'HEVC'

                    _ => {
                        // Try to decode as FourCC
                        let bytes = data1.to_le_bytes();
                        if bytes
                            .iter()
                            .all(|&b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-')
                        {
                            let fourcc: String = bytes.iter().map(|&b| b as char).collect();
                            fourcc.trim().to_string()
                        } else {
                            s.to_string() // Return original if can't decode
                        }
                    }
                };
            }
        }
    }

    // If it's already a readable name, return as-is
    let upper = s.to_ascii_uppercase();
    if upper.contains("VORBIS") {
        return "Vorbis".to_string();
    }
    if upper == "DX50" {
        return "DX50".to_string();
    }
    if upper.contains("DX50") || upper.contains("DIVX") {
        return "DivX".to_string();
    }
    if upper.contains("XVID") {
        return "XviD".to_string();
    }

    s.to_string()
}

/// Check if a codec string is actually a container name (not a real codec)
fn is_container_name(codec: &str, path: &Path) -> bool {
    let codec_lower = codec.to_lowercase();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    // Check if codec matches container extension
    if codec_lower == ext {
        return true;
    }

    // Known container names that aren't real codecs
    matches!(
        codec_lower.as_str(),
        "mkv"
            | "webm"
            | "mp4"
            | "avi"
            | "mov"
            | "wmv"
            | "flv"
            | "video"
            | "audio"
            | "matroska"
            | "container"
    )
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Option<Self> {
        // SAFETY: CoInitializeEx/CoUninitialize balance via RAII; RPC_E_CHANGED_MODE means COM already initialized.
        unsafe {
            let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
            if hr == RPC_E_CHANGED_MODE {
                return Some(Self { initialized: false });
            }
            if hr.is_err() {
                return None;
            }
            Some(Self { initialized: true })
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

unsafe fn open_property_store(path: &Path) -> Result<IPropertyStore, windows::core::Error> {
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: wide_path is a null-terminated UTF-16 buffer that stays alive for the call.
    // GPS_READWRITE: Forces Windows to query installed codecs (K-Lite, etc.) even if not indexed
    // This is the SAME method Windows Explorer uses - slower but gets real codec info
    SHGetPropertyStoreFromParsingName(
        PCWSTR(wide_path.as_ptr()),
        None,
        GETPROPERTYSTOREFLAGS(GPS_READWRITE.0 | GPS_OPENSLOWITEM.0),
    )
}

// EXIF helper: Convert raw F-number value to f-stop string
unsafe fn read_f_number(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    eprintln!("    [DEBUG] F-number raw value: {}", raw);
    // Windows stores as double (e.g., 2.5 for f/2.5)
    Some(format!("f/{:.1}", raw))
}

// EXIF helper: Convert exposure time to 1/x format
unsafe fn read_exposure_time(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    eprintln!("    [DEBUG] Exposure time raw value: {}", raw);
    if raw == 0.0 {
        return None;
    }
    // Windows stores as decimal (e.g., 0.0125 for 1/80)
    if raw < 1.0 {
        Some(format!("1/{} sec.", (1.0 / raw).round() as u32))
    } else {
        Some(format!("{:.2} sec.", raw))
    }
}

// EXIF helper: Focal length in mm
unsafe fn read_focal_length(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    eprintln!("    [DEBUG] Focal length raw value: {}", raw);
    // Windows stores as double (e.g., 7.0 for 7mm)
    Some(format!("{:.0} mm", raw))
}

// EXIF helper: Max aperture F-number
unsafe fn read_aperture(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    eprintln!("    [DEBUG] Max aperture raw value: {}", raw);
    // Windows stores as double
    Some(format!("{:.1}", raw))
}

// EXIF helper: Metering mode friendly name
unsafe fn read_metering_mode(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_u32(store, key)?;
    let mode_name = match raw {
        0 => "Unknown",
        1 => "Average",
        2 => "Center Weighted",
        3 => "Spot",
        4 => "Multi-spot",
        5 => "Pattern",
        6 => "Partial",
        _ => "Other",
    };
    Some(mode_name.to_string())
}

// EXIF helper: Flash mode friendly name
unsafe fn read_flash_mode(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_u32(store, key)?;
    let flash_name = match raw {
        0 => "No flash, compulsory",
        1 => "Flash fired",
        5 => "Flash fired, return light not detected",
        7 => "Flash fired, return light detected",
        8 => "No flash, return light detected",
        16 => "No flash, compulsory",
        24 => "No flash, auto",
        32 => "Flash fired, auto",
        _ => "Unknown flash mode",
    };
    Some(flash_name.to_string())
}

// Helper to read property value as u32
unsafe fn read_u32(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u32> {
    // Get the property value for the key
    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    // Access the raw PROPVARIANT structure
    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    let value = match vt {
        VT_UI4 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal),
        VT_I4 => Some(raw.Anonymous.Anonymous.Anonymous.lVal as u32),
        VT_UI2 => Some(raw.Anonymous.Anonymous.Anonymous.uiVal as u32),
        VT_I2 => Some(raw.Anonymous.Anonymous.Anonymous.iVal as u32),
        0 => None, // VT_EMPTY
        other => {
            eprintln!("    [DEBUG] Unexpected VT type for u32: {}", other);
            None
        }
    };

    value
}

// Helper to read property value as u64
unsafe fn read_u64(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<u64> {
    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    let value = match vt {
        VT_UI8 => Some(raw.Anonymous.Anonymous.Anonymous.uhVal as u64),
        VT_I8 => Some(raw.Anonymous.Anonymous.Anonymous.hVal as u64),
        VT_UI4 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal as u64),
        VT_I4 => Some(raw.Anonymous.Anonymous.Anonymous.lVal as u64),
        _ => None,
    };

    value
}

// Helper to read property value as f64 (double)
unsafe fn read_f64(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<f64> {
    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    let value = match vt {
        VT_R8 => Some(raw.Anonymous.Anonymous.Anonymous.dblVal),
        VT_UI4 => Some(raw.Anonymous.Anonymous.Anonymous.ulVal as f64),
        VT_I4 => Some(raw.Anonymous.Anonymous.Anonymous.lVal as f64),
        _ => {
            eprintln!("    [DEBUG] Unexpected VT type for f64: {}", vt);
            None
        }
    };

    value
}

#[allow(dead_code)]
unsafe fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    const VT_LPWSTR: u16 = 31;
    const VT_BSTR: u16 = 8;
    const VT_EMPTY: u16 = 0;

    let pv = match store.GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
        fmtid: key.fmtid,
        pid: key.pid,
    }) {
        Ok(v) => v,
        Err(_) => return None,
    };

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_EMPTY => None, // Property not available - don't log
        VT_LPWSTR => {
            let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                let slice = std::slice::from_raw_parts(ptr, len);
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        VT_BSTR => {
            // BSTR is also a wide string, try to read it
            let ptr = raw.Anonymous.Anonymous.Anonymous.bstrVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                let slice = std::slice::from_raw_parts(ptr, len);
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        4127 => {
            // VT_VECTOR | VT_LPWSTR (0x1000 | 31)
            // Multi-stream files often store names in vectors
            let c_elems = unsafe { raw.Anonymous.Anonymous.Anonymous.calpwstr.cElems };
            let p_elems = unsafe { raw.Anonymous.Anonymous.Anonymous.calpwstr.pElems };
            if c_elems > 0 && !p_elems.is_null() {
                let mut result = String::new();
                for i in 0..c_elems {
                    let ptr = *p_elems.add(i as usize);
                    if !ptr.is_null() {
                        let len = (0..).take_while(|&j| *ptr.add(j) != 0).count();
                        let slice = std::slice::from_raw_parts(ptr, len);
                        let s = String::from_utf16_lossy(slice);
                        if !result.is_empty() {
                            result.push_str(", ");
                        }
                        result.push_str(&s);
                    }
                }
                if result.is_empty() {
                    None
                } else {
                    Some(result)
                }
            } else {
                None
            }
        }
        other => {
            if other != 0 {
                // Log unexpected VT types for debugging
                eprintln!(
                    "[DEBUG] read_string: unexpected VT type {} for PKEY {{pid={}}}",
                    other, key.pid
                );
            }
            None
        }
    }
}

// Helper to read FourCC (can be u32 or string)
// FourCC is a 4-character code stored as u32 (e.g., 0x31637661 = "avc1")
unsafe fn read_fourcc(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    const VT_LPWSTR: u16 = 31;

    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    match vt {
        VT_UI4 => {
            // FourCC as u32 - convert bytes to ASCII string
            let fourcc = raw.Anonymous.Anonymous.Anonymous.ulVal;
            let bytes = [
                (fourcc & 0xFF) as u8,
                ((fourcc >> 8) & 0xFF) as u8,
                ((fourcc >> 16) & 0xFF) as u8,
                ((fourcc >> 24) & 0xFF) as u8,
            ];
            // Reverse byte order for little-endian and convert to string
            let codec_str = String::from_utf8(bytes.to_vec()).ok()?;
            if codec_str.trim().is_empty() {
                None
            } else {
                Some(codec_str)
            }
        }
        VT_LPWSTR => {
            // FourCC as string (some property handlers use this)
            let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
            if !ptr.is_null() {
                let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
                let slice = std::slice::from_raw_parts(ptr, len);
                Some(String::from_utf16_lossy(slice))
            } else {
                None
            }
        }
        _ => None,
    }
}
