//! Media metadata extraction via Windows Property Store and image headers
//! Follows .cursorrules: single responsibility, < 300 lines

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use image::ImageReader;
use windows::{
    core::{GUID, PCWSTR},
    Win32::Foundation::RPC_E_CHANGED_MODE,
    Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    Win32::UI::Shell::PropertiesSystem::{
        IPropertyStore, SHGetPropertyStoreFromParsingName, GPS_DEFAULT,
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

// PROPVARIANT type tags (from WTypes.h)
const VT_UI4: u16 = 19;
const VT_UI8: u16 = 21;
const VT_I4: u16 = 3;
const VT_I8: u16 = 20;

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
    matches!(
        ext,
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "bmp"
            | "webp"
            | "tiff"
            | "tif"
            | "ico"
            | "heic"
            | "heif"
            | "avif"
    )
}

fn is_video_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp4"
            | "mkv"
            | "avi"
            | "mov"
            | "wmv"
            | "flv"
            | "webm"
            | "m4v"
            | "mpg"
            | "mpeg"
            | "3gp"
            | "ts"
    )
}

fn read_image_metadata(path: &Path) -> Result<MediaMetadata, image::ImageError> {
    // Uses image crate headers only; does not decode the full image.
    let reader = ImageReader::open(path)?;
    let reader = reader.with_guessed_format()?;
    let format_label = reader.format().map(|f| format!("{:?}", f).to_uppercase());
    let (width, height) = reader.into_dimensions()?;

    Ok(MediaMetadata {
        width: Some(width),
        height: Some(height),
        duration_100ns: None,
        frame_rate: None,
        bitrate: None,
        format: format_label,
        color_depth: None,
    })
}

fn read_video_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    let _com_guard = ComGuard::new();
    let store = unsafe { open_property_store(path)? };

    let width = unsafe { read_u32(&store, &PKEY_VIDEO_FRAMEWIDTH) };
    let height = unsafe { read_u32(&store, &PKEY_VIDEO_FRAMEHEIGHT) };
    let duration_100ns = unsafe { read_u64(&store, &PKEY_MEDIA_DURATION) };
    let frame_rate =
        unsafe { read_u32(&store, &PKEY_VIDEO_FRAMERATE) }.map(|raw| raw as f32 / 1_000.0); // documented as 1000 * fps

    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.to_uppercase());

    Ok(MediaMetadata {
        width,
        height,
        duration_100ns,
        frame_rate,
        bitrate: None,
        format,
        color_depth: None,
    })
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
    SHGetPropertyStoreFromParsingName(PCWSTR(wide_path.as_ptr()), None, GPS_DEFAULT)
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
        _ => None,
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

#[allow(dead_code)]
unsafe fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    const VT_LPWSTR: u16 = 31;

    let pv = store
        .GetValue(&windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: key.fmtid,
            pid: key.pid,
        })
        .ok()?;

    let raw = pv.as_raw();
    let vt = raw.Anonymous.Anonymous.vt;

    if vt == VT_LPWSTR {
        let ptr = raw.Anonymous.Anonymous.Anonymous.pwszVal;
        if !ptr.is_null() {
            let len = (0..).take_while(|&i| *ptr.add(i) != 0).count();
            let slice = std::slice::from_raw_parts(ptr, len);
            Some(String::from_utf16_lossy(slice))
        } else {
            None
        }
    } else {
        None
    }
}
