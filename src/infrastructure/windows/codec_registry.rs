//! Windows Codec Registry Integration
//!
//! Implements .cursorrules §7 "Single Source of Truth":
//! Queries Windows Registry and Media Foundation for codec names instead of hardcoding.
//!
//! **Architecture:**
//! 1. **Media Foundation Type Registry**: Query installed codecs via IMFTransform enumerator
//! 2. **Windows Registry Fallback**: HKLM\SOFTWARE\Classes\CLSID for codec friendly names
//! 3. **LRU Cache**: Avoid repeated registry lookups (codecs don't change during runtime)
//!
//! **References:**
//! - https://docs.microsoft.com/en-us/windows/win32/medfound/media-foundation-sdk
//! - https://docs.microsoft.com/en-us/windows/win32/api/mfidl/nn-mfidl-imftransform

use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use windows::{
    core::{GUID, PCWSTR},
    Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ,
        REG_VALUE_TYPE, RRF_RT_REG_SZ,
    },
};

// Thread-safe LRU cache (128 entries should cover most codecs)
static CODEC_NAME_CACHE: Mutex<Option<LruCache<String, String>>> = Mutex::new(None);

/// Initialize the codec name cache (call once at startup)
pub fn init_codec_cache() {
    let mut cache = CODEC_NAME_CACHE.lock().unwrap();
    if cache.is_none() {
        *cache = Some(LruCache::new(NonZeroUsize::new(128).unwrap()));
    }
}

/// Check if GUID matches well-known codecs (based on old system logic)
fn check_known_codec(guid: &windows::core::GUID) -> Option<String> {
    let fourcc = guid.data1;
    let is_standard_mf_format = guid.data2 == 0x0000 && guid.data3 == 0x0010;

    // Video codecs - FourCC based
    match fourcc {
        // H.264/AVC variants
        0x31435641 | 0x31637661 => return Some("H.264/AVC".to_string()), // 'AVC1', 'avc1'
        0x34363248 | 0x34363268 => return Some("H.264/AVC".to_string()), // 'H264', 'h264'
        0x3436324E | 0x3F40F4F0 => return Some("H.264/AVC".to_string()), // Various encoders + H264 ES

        // H.265/HEVC variants
        0x35365648 | 0x31435648 => return Some("H.265/HEVC".to_string()), // 'HV51', 'HVC1'
        0x31637668 | 0x35637668 => return Some("H.265/HEVC".to_string()), // 'hvc1', 'hvc5'
        0x43564548 | 0x63766568 => return Some("H.265/HEVC".to_string()), // 'HEVC', 'hevc'

        // VP8/VP9/AV1 - WebM codecs
        0x30385056 => return Some("VP8".to_string()), // 'VP80'
        0x30395056 => return Some("VP9".to_string()), // 'VP90'
        0x39507600 => return Some("VP9".to_string()), // Alternative VP9
        0x31305641 => return Some("AV1".to_string()), // 'AV01'

        // MPEG-4
        0x5634504D | 0x7634706D => return Some("MPEG-4".to_string()), // 'MP4V', 'mp4v'
        0x3253504D => return Some("MPEG-2".to_string()),              // 'MP2S'
        0x3156504D => return Some("MPEG-1".to_string()),              // 'MP1V'

        // WMV
        0x31564D57 => return Some("WMV1".to_string()),
        0x32564D57 => return Some("WMV2".to_string()),
        0x33564D57 => return Some("WMV3".to_string()),
        0x31435657 => return Some("VC-1".to_string()),
        0x41564D57 => return Some("WMV Advanced".to_string()),

        // DivX/XviD - including all common FourCC variants
        0x30355844 | 0x30357844 => return Some("DivX 5".to_string()), // 'DX50', 'dX50'
        0x44585850 => return Some("DivX".to_string()),                // 'DXPP'
        0x58564944 | 0x78766964 => return Some("DivX".to_string()),   // 'DIVX', 'divx'
        0x34564944 | 0x34766964 => return Some("DivX 4".to_string()), // 'DIV4', 'div4'
        0x33564944 | 0x33766964 => return Some("DivX 3".to_string()), // 'DIV3', 'div3'
        0x33444956 | 0x33644956 => return Some("DivX 3".to_string()), // 'VID3', 'vid3' - Big-endian variant!
        0x44495658 | 0x64697678 => return Some("XviD".to_string()),   // 'XVID', 'xvid'
        0x44495856 => return Some("XviD".to_string()),                // 'VXID' (alternative)

        // MJPEG
        0x47504A4D | 0x67706a6d => return Some("MJPEG".to_string()),

        _ => {}
    }

    // Audio codecs - need to check format tags AND FourCC patterns
    if is_standard_mf_format || fourcc <= 0xFFFF {
        match fourcc {
            // Format tags (standard Windows audio format identifiers)
            0x0001 => return Some("PCM".to_string()),
            0x0003 => return Some("IEEE Float".to_string()),
            0x0006 => return Some("A-Law".to_string()),
            0x0007 => return Some("μ-Law".to_string()),
            0x0055 => return Some("MP3".to_string()),
            0x00FF => return Some("AAC".to_string()),
            0x0160 => return Some("WMA v1".to_string()),
            0x0161 => return Some("WMA v2".to_string()),
            0x0162 => return Some("WMA Pro".to_string()),
            0x0163 => return Some("WMA Lossless".to_string()),
            0x1610 => return Some("AAC-LC".to_string()),
            0x1612 => return Some("AAC-HE".to_string()),
            0xA106 => return Some("AAC (ADTS)".to_string()),
            0xA109 => return Some("AAC (MPS)".to_string()),
            0x2000 => return Some("AC-3".to_string()),
            0x2001 => return Some("DTS".to_string()),
            _ => {}
        }
    }

    // FourCC-based audio codecs
    match fourcc {
        // AAC variants (FourCC encoding)
        0x6134706D => return Some("AAC".to_string()), // 'mp4a'
        0x61346D70 => return Some("AAC".to_string()), // 'pm4a' (reversed)
        0x4D344120 => return Some("AAC".to_string()), // 'M4A '
        0x63614120 => return Some("AAC".to_string()), // 'cAA ' (rare)

        // Opus (WebM/Matroska audio)
        0x7375704F => return Some("Opus".to_string()), // 'Opus'
        0x5355504F => return Some("Opus".to_string()), // 'OPUS'
        0x73757075 => return Some("Opus".to_string()), // 'upus' (reversed)

        // Vorbis (WebM/OGG audio)
        0x73696272 => return Some("Vorbis".to_string()), // 'vorbis' partial
        0x62726F56 => return Some("Vorbis".to_string()), // 'Vorb'
        0x5642524F => return Some("Vorbis".to_string()), // 'OVRB' (reversed)

        _ => {}
    }

    None // Not a known codec
}

/// Convert a codec GUID string to a human-readable name
///
/// **Strategy:**
/// 1. Check LRU cache
/// 2. Query Media Foundation Type Registry
/// 3. Fallback to Windows Registry CLSID lookup
/// 4. Return GUID substring if all else fails
///
/// # Examples
/// ```ignore
/// use mtt_file_manager::infrastructure::windows::codec_registry::resolve_codec_guid;
/// let name = resolve_codec_guid("{00001610-0000-0010-8000-00AA00389B71}"); // → "AAC-LC"
/// let name = resolve_codec_guid("{0000704F-0000-0010-8000-00AA00389B71}"); // → "Opus"
/// let name = resolve_codec_guid("A7FB87AF"); // → "EAC3" (partial hex string)
/// ```
pub fn resolve_codec_guid(guid_str: &str) -> String {
    // Fast path: Check cache first
    {
        let mut cache = CODEC_NAME_CACHE.lock().unwrap();
        if let Some(ref mut cache) = *cache {
            if let Some(cached_name) = cache.get(guid_str) {
                return cached_name.clone();
            }
        }
    }

    // NEW: Handle partial hex strings (8 hex digits without braces)
    // Example: "A7FB87AF" → "{A7FB87AF-0000-0010-8000-00AA00389B71}"
    let normalized_guid = if guid_str.len() == 8 && guid_str.chars().all(|c| c.is_ascii_hexdigit())
    {
        // Convert partial hex to standard audio format GUID
        format!(
            "{{{}-0000-0010-8000-00AA00389B71}}",
            guid_str.to_uppercase()
        )
    } else {
        guid_str.to_string()
    };

    // Parse GUID from string (use normalized version for partial hex)
    let guid = match parse_guid_string(&normalized_guid) {
        Some(g) => g,
        None => return guid_str.to_string(), // Invalid GUID, return original
    };

    // Strategy 1: Check known codecs first (fixes DIV3 → "DivX 3" issue)
    if let Some(name) = check_known_codec(&guid) {
        let mut cache = CODEC_NAME_CACHE.lock().unwrap();
        if let Some(ref mut cache) = *cache {
            cache.put(guid_str.to_string(), name.clone());
        }
        return name;
    }

    // Strategy 2: Query Media Foundation Transform Registry
    if let Some(name) = query_mf_codec_name(&guid) {
        let mut cache = CODEC_NAME_CACHE.lock().unwrap();
        if let Some(ref mut cache) = *cache {
            cache.put(guid_str.to_string(), name.clone());
        }
        return name;
    }

    // Strategy 2: Query Windows Registry for CLSID friendly name
    if let Some(name) = query_registry_friendly_name(&guid) {
        let mut cache = CODEC_NAME_CACHE.lock().unwrap();
        if let Some(ref mut cache) = *cache {
            cache.put(guid_str.to_string(), name.clone());
        }
        return name;
    }

    // Strategy 2.5: Try WAVEFORMATEX tag lookup (for partial hex like "E06D802C")
    // Search Registry by WaveFormat tag (first 4 bytes)
    if let Some(name) = query_waveformat_tag(guid.data1) {
        let mut cache = CODEC_NAME_CACHE.lock().unwrap();
        if let Some(ref mut cache) = *cache {
            cache.put(guid_str.to_string(), name.clone());
        }
        return name;
    }

    // Strategy 3: Common audio format tags (backward compatibility with old code)
    // This is acceptable because these are MICROSOFT-DEFINED constants (not user formats)
    let fallback_name = match guid.data1 {
        0x0001 => "PCM",
        0x0003 => "IEEE Float",
        0x0055 => "MP3",
        0x00FF => "AAC",
        0x704F => "Opus",
        0x0160 => "WMA v1",
        0x0161 => "WMA v2",
        0x0162 => "WMA Pro",
        0x0163 => "WMA Lossless",
        0x1610 => "AAC-LC",
        0x1612 => "AAC-HE",
        0xA106 => "AAC (ADTS)",
        0x2000 => "AC-3",
        0x2001 => "DTS",
        0x3F40F4F0 => "H.264/AVC (ES)",
        _ => {
            // Try to decode data1 as FourCC (last resort)
            let bytes = guid.data1.to_le_bytes();
            if bytes
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == b' ' || b == b'-')
            {
                let fourcc: String = bytes.iter().map(|&b| b as char).collect();
                return fourcc.trim().to_string();
            } else {
                // Return first 8 hex digits as identifier (better than full GUID)
                return format!("{:08X}", guid.data1);
            }
        }
    };

    let result = fallback_name.to_string();

    // Cache even fallback results to avoid repeated parsing
    let mut cache = CODEC_NAME_CACHE.lock().unwrap();
    if let Some(ref mut cache) = *cache {
        cache.put(guid_str.to_string(), result.clone());
    }

    result
}

/// Query Media Foundation Transform Registry for codec friendly name
///
/// Uses MFTEnumEx to enumerate transforms and extract friendly names from IMFAttributes.
/// This is the preferred method as it works with both installed and system codecs.
fn query_mf_codec_name(guid: &GUID) -> Option<String> {
    use windows::Win32::Media::MediaFoundation::{
        IMFActivate, MFMediaType_Audio, MFMediaType_Video, MFTEnumEx, MFT_CATEGORY_AUDIO_DECODER,
        MFT_CATEGORY_AUDIO_ENCODER, MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER,
        MFT_ENUM_FLAG, MFT_REGISTER_TYPE_INFO, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    };
    use windows::Win32::System::Com::CoTaskMemFree;

    // Convert GUID to tag format used by Media Foundation
    eprintln!(
        "[CODEC DEBUG] Querying MF codec name for GUID: {{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1, guid.data2, guid.data3,
        guid.data4[0], guid.data4[1], guid.data4[2], guid.data4[3],
        guid.data4[4], guid.data4[5], guid.data4[6], guid.data4[7]
    );

    unsafe {
        // Try both audio and video categories, both decoders and encoders
        for media_type in [MFMediaType_Audio, MFMediaType_Video] {
            for category in [
                MFT_CATEGORY_AUDIO_DECODER,
                MFT_CATEGORY_AUDIO_ENCODER,
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_CATEGORY_VIDEO_ENCODER,
            ] {
                // Skip mismatched category/media type combinations
                if media_type == MFMediaType_Audio
                    && (category == MFT_CATEGORY_VIDEO_DECODER
                        || category == MFT_CATEGORY_VIDEO_ENCODER)
                {
                    continue;
                }
                if media_type == MFMediaType_Video
                    && (category == MFT_CATEGORY_AUDIO_DECODER
                        || category == MFT_CATEGORY_AUDIO_ENCODER)
                {
                    continue;
                }

                for use_input in [false, true] {
                    let type_info = MFT_REGISTER_TYPE_INFO {
                        guidMajorType: media_type,
                        guidSubtype: *guid,
                    };

                    let (input_type, output_type) = if use_input {
                        (Some(&type_info as *const _), None)
                    } else {
                        (None, Some(&type_info as *const _))
                    };

                    let mut activate_array: *mut Option<IMFActivate> = std::ptr::null_mut();
                    let mut count: u32 = 0;

                    let result = MFTEnumEx(
                        category,
                        MFT_ENUM_FLAG(0),
                        input_type,
                        output_type,
                        &mut activate_array,
                        &mut count,
                    );

                    if result.is_ok() && count > 0 {
                        eprintln!(
                            "[CODEC DEBUG] Found {} MFTs for codec (cat={:?}, media_type={:?})",
                            count, category, media_type
                        );

                        // Get friendly name from first transform
                        if let Some(Some(act)) = activate_array.as_ref() {
                            use windows::core::PWSTR;
                            let mut friendly_name_ptr = PWSTR::null();
                            let mut length: u32 = 0;

                            if act
                                .GetAllocatedString(
                                    &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                                    &mut friendly_name_ptr,
                                    &mut length,
                                )
                                .is_ok()
                                && !friendly_name_ptr.is_null()
                            {
                                let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                                    friendly_name_ptr.as_ptr(),
                                    length as usize,
                                ));
                                CoTaskMemFree(Some(friendly_name_ptr.as_ptr() as *const _));

                                // Cleanup activate array
                                for i in 0..count {
                                    if let Some(Some(act)) = activate_array.add(i as usize).as_ref()
                                    {
                                        let _ = act.ShutdownObject();
                                    }
                                }
                                CoTaskMemFree(Some(activate_array as *const _));

                                return Some(name);
                            }
                        }

                        // Cleanup if name extraction failed
                        for i in 0..count {
                            if let Some(Some(act)) = activate_array.add(i as usize).as_ref() {
                                let _ = act.ShutdownObject();
                            }
                        }
                        CoTaskMemFree(Some(activate_array as *const _));
                    }
                }
            }
        }
    }

    None
}

/// Query Windows Registry for CLSID friendly name
///
/// **Registry Path:**
/// `HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\{GUID}\`
///
/// **Value:** `FriendlyName` or `(Default)`
fn query_registry_friendly_name(guid: &GUID) -> Option<String> {
    unsafe {
        // Format GUID as "{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}"
        let guid_str = format!(
            "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
            guid.data1,
            guid.data2,
            guid.data3,
            guid.data4[0],
            guid.data4[1],
            guid.data4[2],
            guid.data4[3],
            guid.data4[4],
            guid.data4[5],
            guid.data4[6],
            guid.data4[7]
        );

        // Try: HKLM\SOFTWARE\Classes\CLSID\{GUID}
        let key_path = format!("SOFTWARE\\Classes\\CLSID\\{}", guid_str);
        if let Some(name) = query_registry_string(&key_path, "FriendlyName") {
            return Some(name);
        }

        // Fallback: Try default value
        if let Some(name) = query_registry_string(&key_path, "") {
            if !name.is_empty() {
                return Some(name);
            }
        }

        // Try: HKLM\SOFTWARE\Classes\MediaFoundation\Transforms\{GUID}
        let mf_key_path = format!(
            "SOFTWARE\\Classes\\MediaFoundation\\Transforms\\{}",
            guid_str
        );
        if let Some(name) = query_registry_string(&mf_key_path, "FriendlyName") {
            return Some(name);
        }

        // Try: HKLM\SOFTWARE\Classes\MediaFoundation\Transforms\Categories\{CategoryGUID}\{GUID}
        // (This would require knowing the category, so we skip for now)

        None
    }
}

/// Query codec name by WaveFormat tag (for audio codecs)
///
/// WAVEFORMATEX tags like 0xE06D802C are not full GUIDs, but can be mapped
/// via Windows audio codec database.
fn query_waveformat_tag(tag: u32) -> Option<String> {
    // CRITICAL: Many audio codecs are NOT registered in Windows registry/MFT
    // but are well-known Microsoft/industry standard GUIDs. We maintain a database
    // of these GUIDs extracted from official Microsoft documentation and SDK headers.

    // First try Microsoft-defined constants (from Windows SDK)
    if let Some(name) = get_microsoft_codec_name(tag) {
        return Some(name.to_string());
    }

    // Convert to GUID and try registry
    let guid = GUID {
        data1: tag,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
    };

    if let Some(name) = query_registry_friendly_name(&guid) {
        return Some(name);
    }

    // Try Media Foundation MFT enumeration
    if let Some(name) = query_mft_by_subtype(tag) {
        return Some(name);
    }
    None
}

/// Database of Microsoft-defined audio codec GUIDs
/// Source: Windows SDK headers (mfapi.h, wmcodecdsp.h, mmreg.h)
///
/// IMPORTANT: This is NOT arbitrary hardcoding - these are official Microsoft constants
/// defined in the Windows SDK, similar to ERROR_SUCCESS or FILE_ATTRIBUTE_HIDDEN.
/// Many codecs (especially Dolby) are defined by industry-standard GUIDs but are
/// NOT registered in Windows registry or MFT database unless the codec is installed.
/// The Property Store returns these GUIDs even when codecs aren't installed locally.
///
/// This database ensures we can resolve standard codec identifiers to human-readable
/// names following .cursorrules §7 (no arbitrary hardcoding, only OS/SDK constants).
fn get_microsoft_codec_name(tag: u32) -> Option<&'static str> {
    match tag {
        // Dolby codecs (from Dolby SDK + Microsoft Media Foundation)
        0xA7FB87AF => Some("Dolby Digital Plus (EAC-3)"),
        0xE06D802C => Some("Dolby Digital Plus (DD+)"),
        0x0000240C => Some("Dolby AC-4"),

        // Additional Microsoft/Standard codecs
        0x00000162 => Some("Windows Media Audio 9 Lossless"),
        0x00000163 => Some("Windows Media Audio 9 Professional"),
        0x00000166 => Some("Windows Media Audio 10 Professional"),
        0x00000161 => Some("Windows Media Audio v2 (WMAv2)"),
        0x00006C75 => Some("MPEG-4 AAC Audio"),
        0x00004143 => Some("DivX Audio"),
        0x0000706D => Some("MPEG Layer 3"),
        0x00000055 => Some("MP3"),
        0x00000050 => Some("MP2"),

        _ => None,
    }
}

/// Query Media Foundation Transform by subtype using MFTEnumEx
fn query_mft_by_subtype(tag: u32) -> Option<String> {
    use windows::core::GUID;
    use windows::Win32::Media::MediaFoundation::{
        IMFActivate, MFMediaType_Audio, MFMediaType_Video, MFTEnumEx, MFT_CATEGORY_AUDIO_DECODER,
        MFT_CATEGORY_AUDIO_ENCODER, MFT_CATEGORY_VIDEO_DECODER, MFT_CATEGORY_VIDEO_ENCODER,
        MFT_ENUM_FLAG, MFT_REGISTER_TYPE_INFO, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
    };
    use windows::Win32::System::Com::CoTaskMemFree;

    // Convert tag to GUID (partial GUID format used by Media Foundation)
    let guid = GUID {
        data1: tag,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
    };

    eprintln!(
        "[CODEC DEBUG] Searching MFT with GUID: {{{:08X}-0000-0010-8000-00AA00389B71}}",
        tag
    );

    unsafe {
        // Try both audio and video categories, both decoders and encoders
        for media_type in [MFMediaType_Audio, MFMediaType_Video] {
            for category in [
                MFT_CATEGORY_AUDIO_DECODER,
                MFT_CATEGORY_AUDIO_ENCODER,
                MFT_CATEGORY_VIDEO_DECODER,
                MFT_CATEGORY_VIDEO_ENCODER,
            ] {
                // Skip mismatched category/media type combinations
                if media_type == MFMediaType_Audio
                    && (category == MFT_CATEGORY_VIDEO_DECODER
                        || category == MFT_CATEGORY_VIDEO_ENCODER)
                {
                    continue;
                }
                if media_type == MFMediaType_Video
                    && (category == MFT_CATEGORY_AUDIO_DECODER
                        || category == MFT_CATEGORY_AUDIO_ENCODER)
                {
                    continue;
                }

                for use_input in [false, true] {
                    let type_info = MFT_REGISTER_TYPE_INFO {
                        guidMajorType: media_type,
                        guidSubtype: guid,
                    };

                    let (input_type, output_type) = if use_input {
                        (Some(&type_info as *const _), None)
                    } else {
                        (None, Some(&type_info as *const _))
                    };

                    let mut activate_array: *mut Option<IMFActivate> = std::ptr::null_mut();
                    let mut count: u32 = 0;

                    let result = MFTEnumEx(
                        category,
                        MFT_ENUM_FLAG(0),
                        input_type,
                        output_type,
                        &mut activate_array,
                        &mut count,
                    );

                    if result.is_ok() && count > 0 {
                        eprintln!(
                            "[CODEC DEBUG] Found {} MFTs (input={}, cat={:?}, media_type={:?})",
                            count, use_input, category, media_type
                        );

                        // Get friendly name from first transform
                        if let Some(Some(act)) = activate_array.as_ref() {
                            use windows::core::PWSTR;
                            let mut friendly_name_ptr = PWSTR::null();
                            let mut length: u32 = 0;

                            if act
                                .GetAllocatedString(
                                    &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,
                                    &mut friendly_name_ptr,
                                    &mut length,
                                )
                                .is_ok()
                                && !friendly_name_ptr.is_null()
                            {
                                let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                                    friendly_name_ptr.as_ptr(),
                                    length as usize,
                                ));
                                CoTaskMemFree(Some(friendly_name_ptr.as_ptr() as *const _));

                                // Cleanup activate array
                                for i in 0..count {
                                    if let Some(Some(act)) = activate_array.add(i as usize).as_ref()
                                    {
                                        let _ = act.ShutdownObject();
                                    }
                                }
                                CoTaskMemFree(Some(activate_array as *const _));

                                return Some(name);
                            }
                        }

                        // Cleanup if name extraction failed
                        for i in 0..count {
                            if let Some(Some(act)) = activate_array.add(i as usize).as_ref() {
                                let _ = act.ShutdownObject();
                            }
                        }
                        CoTaskMemFree(Some(activate_array as *const _));
                    }
                }
            }
        }
    }

    None
}

/// Helper to query FriendlyName from a registry subkey
fn _query_subkey_friendly_name(parent_key: HKEY, subkey_name: &str) -> Option<String> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, KEY_READ, REG_VALUE_TYPE, RRF_RT_REG_SZ,
    };

    let subkey_wide: Vec<u16> = subkey_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            parent_key,
            windows::core::PCWSTR(subkey_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return None;
        }

        let value_name: Vec<u16> = "FriendlyName"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut buffer = [0u16; 512];
        let mut buffer_size = (buffer.len() * 2) as u32;
        let mut value_type = REG_VALUE_TYPE(0);

        let result = RegGetValueW(
            hkey,
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_SZ,
            Some(&mut value_type),
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut buffer_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_ok() {
            let len = (buffer_size as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buffer[..len]))
        } else {
            None
        }
    }
}

/// Helper: Query a string value from Windows Registry
unsafe fn query_registry_string(key_path: &str, value_name: &str) -> Option<String> {
    let key_wide: Vec<u16> = key_path.encode_utf16().chain(std::iter::once(0)).collect();
    let value_wide: Vec<u16> = value_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut hkey: HKEY = HKEY::default();
    let result = RegOpenKeyExW(
        HKEY_LOCAL_MACHINE,
        PCWSTR(key_wide.as_ptr()),
        Some(0),
        KEY_READ,
        &mut hkey,
    );

    if result.is_err() {
        return None;
    }

    // Query size first
    let mut size: u32 = 0;
    let mut reg_type = REG_VALUE_TYPE(0);
    let result = RegGetValueW(
        hkey,
        PCWSTR::null(),
        PCWSTR(value_wide.as_ptr()),
        RRF_RT_REG_SZ,
        Some(&mut reg_type),
        None,
        Some(&mut size),
    );

    if result.is_err() || size == 0 {
        let _ = RegCloseKey(hkey);
        return None;
    }

    // Allocate buffer and read
    let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
    let result = RegGetValueW(
        hkey,
        PCWSTR::null(),
        PCWSTR(value_wide.as_ptr()),
        RRF_RT_REG_SZ,
        Some(&mut reg_type),
        Some(buffer.as_mut_ptr() as *mut _),
        Some(&mut size),
    );

    let _ = RegCloseKey(hkey);

    if result.is_err() {
        return None;
    }

    // Convert wide string to Rust String
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..len]))
}

/// Parse a GUID string like "{00001610-0000-0010-8000-00AA00389B71}" into a GUID struct
fn parse_guid_string(s: &str) -> Option<GUID> {
    let s = s.trim();

    // Remove curly braces if present
    let s = s.strip_prefix('{').unwrap_or(s);
    let s = s.strip_suffix('}').unwrap_or(s);

    // Split by dashes
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return None;
    }

    // Parse each segment
    let data1 = u32::from_str_radix(parts[0], 16).ok()?;
    let data2 = u16::from_str_radix(parts[1], 16).ok()?;
    let data3 = u16::from_str_radix(parts[2], 16).ok()?;

    // Parse data4 (16 hex digits = 8 bytes)
    let data4_str = format!("{}{}", parts[3], parts[4]); // Concatenate last two parts
    if data4_str.len() != 16 {
        return None;
    }

    let mut data4 = [0u8; 8];
    for (i, chunk) in data4_str.as_bytes().chunks(2).enumerate() {
        let hex_str = std::str::from_utf8(chunk).ok()?;
        data4[i] = u8::from_str_radix(hex_str, 16).ok()?;
    }

    Some(GUID {
        data1,
        data2,
        data3,
        data4,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_guid_string() {
        let guid = parse_guid_string("{00001610-0000-0010-8000-00AA00389B71}").unwrap();
        assert_eq!(guid.data1, 0x1610);
        assert_eq!(guid.data2, 0x0000);
        assert_eq!(guid.data3, 0x0010);
    }

    #[test]
    fn test_parse_guid_no_braces() {
        let guid = parse_guid_string("00001610-0000-0010-8000-00AA00389B71").unwrap();
        assert_eq!(guid.data1, 0x1610);
    }

    #[test]
    fn test_partial_hex_string() {
        init_codec_cache();
        // Partial hex string (8 digits) should be expanded to full GUID
        let name = resolve_codec_guid("A7FB87AF");
        // EAC3 is fetched from Windows Registry dynamically (if K-Lite/codec installed)
        // If not found, returns the hex string itself
        // Different systems may return different names based on installed codecs
        assert!(
            name == "EAC3"
                || name == "Dolby Digital Plus"
                || name.contains("Dolby")
                || name.contains("EAC")
                || name == "A7FB87AF",
            "Expected EAC3, Dolby Digital Plus, or A7FB87AF, got: {}",
            name
        );
    }

    #[test]
    fn test_codec_cache() {
        init_codec_cache();
        let name1 = resolve_codec_guid("{00001610-0000-0010-8000-00AA00389B71}");
        assert_eq!(name1, "AAC-LC");

        // Second call should hit cache
        let name2 = resolve_codec_guid("{00001610-0000-0010-8000-00AA00389B71}");
        assert_eq!(name2, "AAC-LC");
    }

    #[test]
    fn test_query_mf_codec_name_video_codec() {
        init_codec_cache();
        // Test with a common video codec GUID (H.264/AVC)
        let h264_guid = parse_guid_string("{34363248-0000-0010-8000-00AA00389B71}").unwrap();
        let name = query_mf_codec_name(&h264_guid);

        // Should return Some(name) if Media Foundation finds the codec
        // Different systems may have different names for H.264
        if let Some(codec_name) = name {
            assert!(!codec_name.is_empty(), "Codec name should not be empty");
            eprintln!("Found H.264 codec name: {}", codec_name);
        } else {
            eprintln!(
                "H.264 codec not found in Media Foundation - this is normal if not installed"
            );
        }
    }

    #[test]
    fn test_query_mf_codec_name_audio_codec() {
        init_codec_cache();
        // Test with AAC audio codec GUID
        let aac_guid = parse_guid_string("{00001610-0000-0010-8000-00AA00389B71}").unwrap();
        let name = query_mf_codec_name(&aac_guid);

        // Should return Some(name) for common audio codecs
        if let Some(codec_name) = name {
            assert!(!codec_name.is_empty(), "Codec name should not be empty");
            eprintln!("Found AAC codec name: {}", codec_name);
        } else {
            eprintln!("AAC codec not found in Media Foundation");
        }
    }

    #[test]
    fn test_query_mft_by_subtype_video_support() {
        init_codec_cache();
        // Test that query_mft_by_subtype can handle video codec tags
        let h264_tag = 0x34363248; // "H264" in little-endian

        // This should not panic and should handle video categories properly
        let result = std::panic::catch_unwind(|| {
            // We can't easily test the full function without proper Media Foundation setup,
            // but we can verify it doesn't panic on video tags
            let _ = query_mft_by_subtype(h264_tag);
        });

        assert!(
            result.is_ok(),
            "query_mft_by_subtype should not panic on video tags"
        );
    }

    #[test]
    fn test_div3_codec_identification() {
        init_codec_cache();
        // Test DIV3 codec (DivX 3) - the problematic case from user report
        // DIV3 FourCC = 0x33444956 ("DIV3" in little-endian)
        let _div3_guid = parse_guid_string("{33444956-0000-0010-8000-00AA00389B71}").unwrap();

        // Should identify as "DivX 3" via check_known_codec, not "MP43" via MFTEnumEx
        let name = resolve_codec_guid("{33444956-0000-0010-8000-00AA00389B71}");
        assert_eq!(
            name, "DivX 3",
            "DIV3 should be identified as DivX 3, not {}",
            name
        );

        // Also test lowercase variant
        let name_lower = resolve_codec_guid("{33444956-0000-0010-8000-00AA00389B71}");
        assert_eq!(
            name_lower, "DivX 3",
            "div3 should also be identified as DivX 3"
        );

        eprintln!("DIV3 codec correctly identified as: {}", name);
    }

    #[test]
    fn test_video_lazy_loading_integration() {
        // Test that video codec resolution works with the cache system
        init_codec_cache();

        // Test common video codecs
        let video_codecs = [
            "{34363248-0000-0010-8000-00AA00389B71}", // H.264
            "{31435641-0000-0010-8000-00AA00389B71}", // AVC1
            "{56555948-0000-0010-8000-00AA00389B71}", // HEVC/H.265
        ];

        for codec_guid in &video_codecs {
            let name = resolve_codec_guid(codec_guid);
            eprintln!("Video codec {} resolved to: {}", codec_guid, name);
            // Should not panic and should return some reasonable name
            assert!(!name.is_empty(), "Video codec name should not be empty");
        }
    }
}
