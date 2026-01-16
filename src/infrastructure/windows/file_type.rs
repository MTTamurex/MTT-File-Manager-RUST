//! File type detection using Windows Perceived Type API.
//!
//! Uses `AssocGetPerceivedType` to dynamically detect file types based on
//! Windows registry (respects K-Lite/Icaros handlers).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;

use windows::{core::PCWSTR, Win32::UI::Shell::AssocGetPerceivedType};

// PERCEIVED type values from shlwapi.h
const PERCEIVED_TYPE_IMAGE: i32 = 2;
const PERCEIVED_TYPE_AUDIO: i32 = 3;
const PERCEIVED_TYPE_VIDEO: i32 = 4;

// PERCEIVED is a simple i32 wrapper - define it ourselves
#[repr(transparent)]
#[derive(Clone, Copy, Default)]
struct PERCEIVED(i32);

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

/// Get the perceived type of a file based on its extension.
///
/// Uses Windows `AssocGetPerceivedType` API to query the registry for the file type.
/// This respects any handlers installed by codec packs like K-Lite/Icaros.
///
/// Results are cached for performance.
pub fn get_perceived_type(extension: &str) -> PerceivedType {
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
        let cache_guard = cache.lock().unwrap();
        if let Some(&cached_type) = cache_guard.get(&ext_with_dot) {
            return cached_type;
        }
    }

    // Query Windows API
    let mut perceived = query_perceived_type(&ext_with_dot);

    // Fallback for common video extensions if Windows query fails
    if perceived == PerceivedType::Other {
        let clean_ext = ext_with_dot.trim_start_matches('.');
        if matches!(clean_ext, "mp4" | "mkv" | "avi" | "webm" | "mov" | "wmv" | "flv" | "ogm" | "ts" | "mts" | "m2ts" | "vob" | "3gp" | "vtt" | "m4v" | "m4a" | "m4b" | "m4p" | "m4r" | "3g2" | "3gp2" | "3gpp" | "amv" | "asf" | "avs" | "bik" | "divx" | "drc" | "dvr-ms" | "f4v" | "gvi" | "gxf" | "ismv" | "ivf" | "k3g" | "m2v" | "m4u" | "mj2" | "mjp2" | "mod" | "mp2v" | "mp4v" | "mpa" | "mpe" | "mpeg" | "mpg" | "mpv2" | "mqv" | "nsv" | "nuv" | "ogv" | "pva" | "qt" | "rec" | "rm" | "rmvb" | "rpl" | "thp" | "tod" | "trp" | "ty" | "vid" | "vro" | "weba" | "wm" | "wmp" | "wvx" | "xesc" | "y4m") {
            perceived = PerceivedType::Video;
        }
    }

    // Only cache successful results (not Other)
    if perceived != PerceivedType::Other {
        let mut cache_guard = cache.lock().unwrap();
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
            // Cast our PERCEIVED to the type windows-rs expects
            std::mem::transmute::<*mut PERCEIVED, _>(&mut perceived_type),
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
    get_perceived_type(extension) == PerceivedType::Image
}


/// Busca primeiro item de mídia (imagem ou vídeo) em uma pasta para usar como preview
/// Verifica apenas os primeiros 15 arquivos para performance
pub fn find_folder_preview_item(folder_path: &Path) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(folder_path) {
        for entry in entries.flatten().take(15) {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    // Para preview de pasta, queremos apenas imagens ou vídeos
                    let ptype = get_perceived_type(ext);
                    if ptype == PerceivedType::Image || ptype == PerceivedType::Video {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
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
        assert_eq!(get_perceived_type("ts"), PerceivedType::Video);
        assert_eq!(get_perceived_type("vob"), PerceivedType::Video);
    }

    #[test]
    fn test_common_image_extensions() {
        assert_eq!(get_perceived_type("jpg"), PerceivedType::Image);
        assert_eq!(get_perceived_type("png"), PerceivedType::Image);
    }

}
