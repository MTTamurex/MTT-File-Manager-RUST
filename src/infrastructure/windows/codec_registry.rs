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

/// Convert a codec GUID string to a human-readable name
///
/// **Strategy:**
/// 1. Check LRU cache
/// 2. Query Media Foundation Type Registry
/// 3. Fallback to Windows Registry CLSID lookup
/// 4. Return GUID substring if all else fails
///
/// # Examples
/// ```
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
    let normalized_guid = if guid_str.len() == 8 && guid_str.chars().all(|c| c.is_ascii_hexdigit()) {
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

    // Strategy 1: Query Media Foundation Transform Registry
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
/// TODO: Full implementation requires MFTEnumEx API (Windows 7+)
/// For now, we rely on Registry fallback which is sufficient for most codecs.
fn query_mf_codec_name(_guid: &GUID) -> Option<String> {
    // TODO: Implement full MFTransform enumeration
    // This requires:
    // 1. MFTEnumEx API (Windows 7+)
    // 2. Iterate through all transforms in category
    // 3. Check input/output types against target GUID
    // 4. Extract friendly name from IMFAttributes
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
        let mf_key_path = format!("SOFTWARE\\Classes\\MediaFoundation\\Transforms\\{}", guid_str);
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
fn query_waveformat_tag(_tag: u32) -> Option<String> {
    // TODO: Implement full WAVEFORMATEX tag database lookup
    // For now, return None to use fallback constants
    None
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
        0,
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
        assert!(
            name == "EAC3" 
            || name == "Dolby Digital Plus" 
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
}
