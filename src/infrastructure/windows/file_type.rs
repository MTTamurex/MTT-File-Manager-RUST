//! File type detection using Windows Perceived Type API.
//!
//! Uses `AssocGetPerceivedType` to dynamically detect file types based on
//! Windows registry (respects K-Lite/Icaros handlers).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;
use windows::{
    core::PCWSTR,
    Win32::UI::Shell::{AssocGetPerceivedType, Common::PERCEIVED},
};

// PERCEIVED type values from shlwapi.h
const PERCEIVED_TYPE_IMAGE: i32 = 2;
const PERCEIVED_TYPE_AUDIO: i32 = 3;
const PERCEIVED_TYPE_VIDEO: i32 = 4;

/// Perceived file type category
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerceivedType {
    Video,
    Audio,
    Image,
    Other,
}

/// Cache for perceived types (extension -> type)
static PERCEIVED_TYPE_CACHE: OnceLock<Mutex<HashMap<String, PerceivedType>>> = OnceLock::new();

/// PERFORMANCE: Zero-allocation fast path for common media extensions.
/// Avoids to_lowercase(), format!(), and Mutex lock for the most frequent extensions.
/// Returns None for uncommon extensions → falls through to cached Windows API.
#[inline]
fn get_perceived_type_fast(ext: &str) -> Option<PerceivedType> {
    let bytes = ext.as_bytes();
    // Handle optional leading dot
    let bytes = if bytes.first() == Some(&b'.') {
        &bytes[1..]
    } else {
        bytes
    };

    match bytes.len() {
        2 => {
            let b = [
                bytes[0].to_ascii_lowercase(),
                bytes[1].to_ascii_lowercase(),
            ];
            match &b {
                // Common non-media: avoid API call entirely
                b"db" | b"js" | b"py" | b"cs" | b"rs" | b"md" | b"7z" | b"gz" => {
                    Some(PerceivedType::Other)
                }
                _ => None,
            }
        }
        3 => {
            let b = [
                bytes[0].to_ascii_lowercase(),
                bytes[1].to_ascii_lowercase(),
                bytes[2].to_ascii_lowercase(),
            ];
            match &b {
                // Image
                b"jpg" | b"png" | b"gif" | b"bmp" | b"svg" | b"ico" | b"tga" | b"psd" | b"raw" => {
                    Some(PerceivedType::Image)
                }
                // Video
                b"mp4" | b"mkv" | b"avi" | b"wmv" | b"mov" | b"flv" | b"ogv" | b"ogm" | b"m4v"
                | b"3gp" | b"vob" | b"mts" | b"asf" | b"m2v" | b"mpg" => Some(PerceivedType::Video),
                // Audio
                b"mp3" | b"wav" | b"ogg" | b"wma" | b"aac" | b"m4a" | b"ape" | b"mid" => {
                    Some(PerceivedType::Audio)
                }
                // Common non-media: avoid API call entirely
                b"exe" | b"dll" | b"sys" | b"ini" | b"cfg" | b"log" | b"txt" | b"xml"
                | b"dat" | b"nls" | b"bin" | b"cat" | b"msi" | b"cab" | b"tmp" | b"bat"
                | b"cmd" | b"reg" | b"inf" | b"ttf" | b"otf" | b"zip" | b"rar" | b"lnk"
                | b"url" | b"htm" | b"css" | b"pdf" | b"doc" | b"xls" | b"ppt" | b"rtf"
                | b"msc" | b"cpl" | b"scr" | b"com" | b"drv" | b"ocx" | b"mui" | b"man"
                | b"mof" | b"mum" | b"prx" | b"nfo" | b"ion" | b"rll" | b"tlb" | b"mfl"
                | b"sdb" | b"cur" | b"ani" => {
                    Some(PerceivedType::Other)
                }
                _ => None,
            }
        }
        4 => {
            let b = [
                bytes[0].to_ascii_lowercase(),
                bytes[1].to_ascii_lowercase(),
                bytes[2].to_ascii_lowercase(),
                bytes[3].to_ascii_lowercase(),
            ];
            match &b {
                // Image
                b"jpeg" | b"webp" | b"tiff" | b"avif" | b"heic" | b"heif" | b"jfif" => {
                    Some(PerceivedType::Image)
                }
                // Video
                b"webm" | b"mpeg" | b"m2ts" | b"divx" | b"rmvb" => Some(PerceivedType::Video),
                // Audio
                b"flac" | b"alac" | b"opus" | b"aiff" | b"weba" => Some(PerceivedType::Audio),
                // Common non-media: avoid API call entirely
                b"docx" | b"xlsx" | b"pptx" | b"json" | b"yaml" | b"toml" | b"html"
                | b"lock" | b"conf" | b"java" | b"xaml" => {
                    Some(PerceivedType::Other)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Get the perceived type of a file based on its extension.
///
/// Uses Windows `AssocGetPerceivedType` API to query the registry for the file type.
/// This respects any handlers installed by codec packs like K-Lite/Icaros.
///
/// Results are cached for performance.
pub fn get_perceived_type(extension: &str) -> PerceivedType {
    // PERFORMANCE: Fast path for common extensions (zero allocation, no mutex)
    if let Some(ptype) = get_perceived_type_fast(extension) {
        return ptype;
    }

    // Normalize extension (lowercase, ensure starts with dot)
    let ext_lower = extension.to_lowercase();
    let ext_with_dot = if ext_lower.starts_with('.') {
        ext_lower
    } else {
        format!(".{}", ext_lower)
    };

    // Check cache first
    let cache = PERCEIVED_TYPE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));

    {
        let cache_guard = cache.lock();
        if let Some(&cached_type) = cache_guard.get(&ext_with_dot) {
            return cached_type;
        }
    }

    // Query Windows API
    let mut perceived = query_perceived_type(&ext_with_dot);

    // Fallback for common video extensions if Windows query fails
    if perceived == PerceivedType::Other {
        let clean_ext = ext_with_dot.trim_start_matches('.');
        if matches!(
            clean_ext,
            "mp4"
                | "mkv"
                | "avi"
                | "webm"
                | "mov"
                | "wmv"
                | "flv"
                | "ogm"
                | "mts"
                | "m2ts"
                | "vob"
                | "3gp"
                | "vtt"
                | "m4v"
                | "m4a"
                | "m4b"
                | "m4p"
                | "m4r"
                | "3g2"
                | "3gp2"
                | "3gpp"
                | "amv"
                | "asf"
                | "avs"
                | "bik"
                | "divx"
                | "drc"
                | "dvr-ms"
                | "f4v"
                | "gvi"
                | "gxf"
                | "ismv"
                | "ivf"
                | "k3g"
                | "m2v"
                | "m4u"
                | "mj2"
                | "mjp2"
                | "mod"
                | "mp2v"
                | "mp4v"
                | "mpa"
                | "mpe"
                | "mpeg"
                | "mpg"
                | "mpv2"
                | "mqv"
                | "nsv"
                | "nuv"
                | "ogv"
                | "pva"
                | "qt"
                | "rec"
                | "rm"
                | "rmvb"
                | "rpl"
                | "thp"
                | "tod"
                | "trp"
                | "ty"
                | "vid"
                | "vro"
                | "weba"
                | "wm"
                | "wmp"
                | "wvx"
                | "xesc"
                | "y4m"
        ) {
            perceived = PerceivedType::Video;
        }
    }

    // Cache ALL results (including Other) to avoid repeated Windows API calls.
    // Previously, Other was not cached, causing AssocGetPerceivedType to be called
    // for every non-media file on every frame (2-20ms per call).
    {
        let mut cache_guard = cache.lock();
        cache_guard.insert(ext_with_dot, perceived);
    }

    perceived
}

/// Query Windows for the perceived type of an extension using AssocGetPerceivedType
fn query_perceived_type(extension: &str) -> PerceivedType {
    // Convert to wide string
    let ext_wide: Vec<u16> = OsStr::new(extension)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut perceived_type = PERCEIVED::default();
    let mut perceived_flag: u32 = 0;

    let result = unsafe {
        AssocGetPerceivedType(
            PCWSTR(ext_wide.as_ptr()),
            &mut perceived_type,
            &mut perceived_flag,
            None,
        )
    };

    if result.is_err() {
        return PerceivedType::Other;
    }

    // Map PERCEIVED values to our PerceivedType
    match perceived_type.0 {
        PERCEIVED_TYPE_IMAGE => PerceivedType::Image,
        PERCEIVED_TYPE_AUDIO => PerceivedType::Audio,
        PERCEIVED_TYPE_VIDEO => PerceivedType::Video,
        _ => PerceivedType::Other,
    }
}

/// Check if an extension is a media file (video, audio, or image)
pub fn is_media_extension(extension: &str) -> bool {
    matches!(
        get_perceived_type(extension),
        PerceivedType::Video | PerceivedType::Audio | PerceivedType::Image
    )
}

/// Check if an extension is a video file
pub fn is_video_extension(extension: &str) -> bool {
    get_perceived_type(extension) == PerceivedType::Video
}

/// Check if an extension is an audio file
pub fn is_audio_extension(extension: &str) -> bool {
    get_perceived_type(extension) == PerceivedType::Audio
}

/// Check if an extension is an image file
pub fn is_image_extension(extension: &str) -> bool {
    if extension.eq_ignore_ascii_case("svg") {
        return true;
    }

    get_perceived_type(extension) == PerceivedType::Image
}

/// Finds the first media item (image or video) in a folder to use as a preview.
///
/// Scans up to `MAX_ENTRIES` directory entries total (to avoid slow scans on
/// huge folders), but only counts **files** against the check budget — directories
/// are skipped without counting. This ensures folders with many subfolders but
/// few media files (e.g. 100 subfolders + 1 jpg) still find the media item.
///
/// CRITICAL: Uses timeout-protected enumeration for OneDrive to prevent indefinite blocking.
const MAX_ENTRIES_SCAN: usize = 500;
const MAX_FILES_CHECK: usize = 30;

pub fn find_folder_preview_item(folder_path: &Path) -> Option<PathBuf> {
    use crate::infrastructure::onedrive::{
        is_onedrive_path, onedrive_read_directory, IoTimeoutResult,
    };

    // For OneDrive paths, use timeout-protected enumeration
    // fs::read_dir() can block for 30-60s on cloud-only folders
    if is_onedrive_path(folder_path) {
        match onedrive_read_directory(folder_path) {
            IoTimeoutResult::Ok(entries) => {
                let mut files_checked = 0usize;
                for (idx, (filename, attrs, _, _)) in entries.into_iter().enumerate() {
                    if idx >= MAX_ENTRIES_SCAN {
                        break;
                    }
                    // Skip directories — don't count against budget
                    let is_dir = (attrs & 0x10) != 0; // FILE_ATTRIBUTE_DIRECTORY
                    if is_dir {
                        continue;
                    }
                    files_checked += 1;
                    if files_checked > MAX_FILES_CHECK {
                        break;
                    }
                    let path = folder_path.join(&filename);
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        let ptype = get_perceived_type(ext);
                        if (ptype == PerceivedType::Image || ptype == PerceivedType::Video)
                            && is_valid_media_candidate(&path, ptype)
                        {
                            return Some(path);
                        }
                    }
                }
                return None;
            }
            IoTimeoutResult::Timeout => {
                log::warn!("[COVER] OneDrive folder scan timed out: {:?}", folder_path);
                return None;
            }
            IoTimeoutResult::Err(_) => return None,
        }
    }

    // Standard path (non-OneDrive) - use regular fs::read_dir
    if let Ok(entries) = fs::read_dir(folder_path) {
        let mut files_checked = 0usize;
        for (idx, entry) in entries.flatten().enumerate() {
            if idx >= MAX_ENTRIES_SCAN {
                break;
            }
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            // Skip directories — don't count against budget
            if !file_type.is_file() {
                continue;
            }
            files_checked += 1;
            if files_checked > MAX_FILES_CHECK {
                break;
            }
            let path = entry.path();
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                // For folder preview, we only want images or videos
                let ptype = get_perceived_type(ext);
                if (ptype == PerceivedType::Image || ptype == PerceivedType::Video)
                    && is_valid_media_candidate(&path, ptype)
                {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Check if a `.ts` file is a real MPEG Transport Stream (sync byte 0x47).
/// Returns `false` for TypeScript or unreadable files.
pub fn is_mpeg_ts_file(path: &Path) -> bool {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut first_byte = [0u8; 1];
    if file.read_exact(&mut first_byte).is_err() {
        return false;
    }

    first_byte[0] == 0x47
}

fn is_valid_media_candidate(path: &Path, ptype: PerceivedType) -> bool {
    if ptype != PerceivedType::Video {
        return true;
    }

    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ts"))
    {
        return is_mpeg_ts_file(path);
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_video_extensions() {
        assert_eq!(get_perceived_type("mp4"), PerceivedType::Video);
        assert_eq!(get_perceived_type("mkv"), PerceivedType::Video);
        assert_eq!(get_perceived_type("avi"), PerceivedType::Video);
        assert_eq!(get_perceived_type("ogm"), PerceivedType::Video);
        assert_eq!(get_perceived_type("vob"), PerceivedType::Video);
    }

    #[test]
    fn test_common_image_extensions() {
        assert_eq!(get_perceived_type("jpg"), PerceivedType::Image);
        assert_eq!(get_perceived_type("png"), PerceivedType::Image);
    }
}
