//! MediaFoundation-based video metadata extraction
//!
//! Fallback for Property Store when it fails to retrieve video metadata.
//! Uses IMFSourceReader to read video stream information directly.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::{
    core::PCWSTR,
    Win32::Foundation::RPC_E_CHANGED_MODE,
    Win32::Media::MediaFoundation::{
        IMFMediaType, IMFSourceReader, MFCreateSourceReaderFromURL, MFShutdown, MFStartup,
        MFSTARTUP_NOSOCKET, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
        MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_SUBTYPE, MF_PD_DURATION,
    },
    Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
};

// Stream index constants - these are defined as i32/u32 in the actual API
const MF_SOURCE_READER_FIRST_VIDEO_STREAM: u32 = 0xFFFFFFFC; // -4 as u32
const MF_SOURCE_READER_FIRST_AUDIO_STREAM: u32 = 0xFFFFFFFD; // -3 as u32
const MF_SOURCE_READER_MEDIASOURCE: u32 = 0xFFFFFFFF; // -1 as u32

/// Video metadata extracted via MediaFoundation
#[derive(Debug, Default, Clone)]
pub struct VideoMetadataMF {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_100ns: Option<u64>,
    pub frame_rate_num: Option<u32>,
    pub frame_rate_den: Option<u32>,
    pub video_bitrate: Option<u32>,
    pub video_codec_guid: Option<String>,
    pub audio_channels: Option<u32>,
    pub audio_sample_rate: Option<u32>,
    pub audio_bitrate: Option<u32>,
    pub audio_codec_guid: Option<String>,
}

/// RAII guard for COM initialization
struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Option<Self> {
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

/// RAII guard for MediaFoundation startup/shutdown
struct MFGuard {
    started: bool,
}

impl MFGuard {
    fn new() -> Option<Self> {
        // SAFETY: MFStartup is safe to call multiple times; we track state.
        // MF_VERSION = 0x00020070 (MF 2.0)
        unsafe {
            if MFStartup(0x00020070, MFSTARTUP_NOSOCKET).is_ok() {
                Some(Self { started: true })
            } else {
                None
            }
        }
    }
}

impl Drop for MFGuard {
    fn drop(&mut self) {
        if self.started {
            unsafe {
                let _ = MFShutdown();
            }
        }
    }
}

/// Extract video metadata using MediaFoundation IMFSourceReader.
///
/// This is more reliable than Property Store for modern codecs and containers,
/// but is slower because it opens and parses the file.
pub fn extract_video_metadata_mf(path: &Path) -> Option<VideoMetadataMF> {
    // Skip cloud-only OneDrive files — MFCreateSourceReaderFromURL opens the file
    // and can trigger a download, blocking the thread for 30-60+ seconds.
    if crate::infrastructure::onedrive::is_onedrive_path(path)
        && !crate::infrastructure::onedrive::is_locally_available(path)
    {
        return None;
    }

    let _com = ComGuard::new()?;
    let _mf = MFGuard::new()?;

    // Convert path to wide string
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // Create source reader from URL (file path)
    let reader: IMFSourceReader =
        unsafe { MFCreateSourceReaderFromURL(PCWSTR(wide_path.as_ptr()), None).ok()? };

    let mut meta = VideoMetadataMF::default();

    // Read duration from presentation descriptor
    meta.duration_100ns = read_duration(&reader);

    // Read video stream info
    if let Some(video_type) = get_native_media_type(&reader, MF_SOURCE_READER_FIRST_VIDEO_STREAM) {
        read_video_info(&video_type, &mut meta);
    }

    // Read audio stream info
    if let Some(audio_type) = get_native_media_type(&reader, MF_SOURCE_READER_FIRST_AUDIO_STREAM) {
        read_audio_info(&audio_type, &mut meta);
    }

    Some(meta)
}

/// Get native media type for a stream
fn get_native_media_type(reader: &IMFSourceReader, stream_index: u32) -> Option<IMFMediaType> {
    unsafe { reader.GetNativeMediaType(stream_index, 0).ok() }
}

/// Read duration from the media source's presentation attribute
fn read_duration(reader: &IMFSourceReader) -> Option<u64> {
    unsafe {
        // Get the presentation attribute as PROPVARIANT
        let propvar = reader
            .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE, &MF_PD_DURATION)
            .ok()?;

        // PROPVARIANT for duration contains VT_UI8 (u64)
        // Access the raw value directly from the anonymous union
        let raw = &*(&propvar.Anonymous.Anonymous as *const _
            as *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT_0_0);
        let vt = raw.vt;

        // VT_UI8 = 21, VT_I8 = 20
        match vt.0 {
            21 => Some(raw.Anonymous.uhVal), // VT_UI8
            20 => {
                let val = raw.Anonymous.hVal;
                if val >= 0 {
                    Some(val as u64)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Extract video stream information from media type
fn read_video_info(media_type: &IMFMediaType, meta: &mut VideoMetadataMF) {
    unsafe {
        // Frame size (width/height packed as u64)
        if let Ok(frame_size) = media_type.GetUINT64(&MF_MT_FRAME_SIZE) {
            meta.width = Some((frame_size >> 32) as u32);
            meta.height = Some((frame_size & 0xFFFFFFFF) as u32);
        }

        // Frame rate (num/den packed as u64)
        if let Ok(frame_rate) = media_type.GetUINT64(&MF_MT_FRAME_RATE) {
            meta.frame_rate_num = Some((frame_rate >> 32) as u32);
            meta.frame_rate_den = Some((frame_rate & 0xFFFFFFFF) as u32);
        }

        // Video bitrate
        if let Ok(bitrate) = media_type.GetUINT32(&MF_MT_AVG_BITRATE) {
            meta.video_bitrate = Some(bitrate);
        }

        // Video codec (subtype GUID)
        if let Ok(subtype) = media_type.GetGUID(&MF_MT_SUBTYPE) {
            meta.video_codec_guid = Some(guid_to_codec_name(&subtype));
        }
    }
}

/// Extract audio stream information from media type
fn read_audio_info(media_type: &IMFMediaType, meta: &mut VideoMetadataMF) {
    unsafe {
        // Channel count
        if let Ok(channels) = media_type.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) {
            meta.audio_channels = Some(channels);
        }

        // Sample rate
        if let Ok(sample_rate) = media_type.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) {
            meta.audio_sample_rate = Some(sample_rate);
        }

        // Audio bitrate
        if let Ok(bitrate) = media_type.GetUINT32(&MF_MT_AVG_BITRATE) {
            meta.audio_bitrate = Some(bitrate);
        }

        // Audio codec (subtype GUID)
        if let Ok(subtype) = media_type.GetGUID(&MF_MT_SUBTYPE) {
            meta.audio_codec_guid = Some(guid_to_codec_name(&subtype));
        }
    }
}

/// Convert well-known MediaFoundation subtype GUIDs to human-readable codec names
fn guid_to_codec_name(guid: &windows::core::GUID) -> String {
    // First, check for well-known full GUIDs (audio codecs often need this)
    // Format: {XXXXXXXX-0000-0010-8000-00AA00389B71} is the standard MediaFoundation format
    // where XXXXXXXX is the FourCC or format tag

    let fourcc = guid.data1;

    // Check if it's a standard MediaFoundation GUID format (data2=0x0000, data3=0x0010)
    // or FourCC-based GUID format
    let is_standard_mf_format = guid.data2 == 0x0000 && guid.data3 == 0x0010;

    // Video codecs - FourCC based
    match fourcc {
        // H.264/AVC variants
        0x31435641 | 0x31637661 => return "H.264/AVC".to_string(), // 'AVC1', 'avc1'
        0x34363248 | 0x34363268 => return "H.264/AVC".to_string(), // 'H264', 'h264'
        0x3436324E | 0x3F40F4F0 => return "H.264/AVC".to_string(), // Various encoders + H264 ES

        // H.265/HEVC variants
        0x35365648 | 0x31435648 => return "H.265/HEVC".to_string(), // 'HV51', 'HVC1'
        0x31637668 | 0x35637668 => return "H.265/HEVC".to_string(), // 'hvc1', 'hvc5'
        0x43564548 | 0x63766568 => return "H.265/HEVC".to_string(), // 'HEVC', 'hevc'

        // VP8/VP9/AV1 - WebM codecs
        0x30385056 => return "VP8".to_string(), // 'VP80'
        0x30395056 => return "VP9".to_string(), // 'VP90'
        0x39507600 => return "VP9".to_string(), // Alternative VP9
        0x31305641 => return "AV1".to_string(), // 'AV01'

        // MPEG-4
        0x5634504D | 0x7634706D => return "MPEG-4".to_string(), // 'MP4V', 'mp4v'
        0x3253504D => return "MPEG-2".to_string(),              // 'MP2S'
        0x3156504D => return "MPEG-1".to_string(),              // 'MP1V'

        // WMV
        0x31564D57 => return "WMV1".to_string(),
        0x32564D57 => return "WMV2".to_string(),
        0x33564D57 => return "WMV3".to_string(),
        0x31435657 => return "VC-1".to_string(),
        0x41564D57 => return "WMV Advanced".to_string(),

        // DivX/XviD - including all common FourCC variants
        // DX50 = DivX 5.0 (0x30355844 = "DX50" as little-endian)
        0x30355844 | 0x30357844 => return "DivX 5".to_string(), // 'DX50', 'dX50'
        0x44585850 => return "DivX".to_string(),                // 'DXPP'
        0x58564944 | 0x78766964 => return "DivX".to_string(),   // 'DIVX', 'divx'
        0x34564944 | 0x34766964 => return "DivX 4".to_string(), // 'DIV4', 'div4'
        0x33564944 | 0x33766964 => return "DivX 3".to_string(), // 'DIV3', 'div3'
        0x33444956 | 0x33644956 => return "DivX 3".to_string(), // 'VID3', 'vid3' - Big-endian variant!
        0x44495658 | 0x64697678 => return "XviD".to_string(),   // 'XVID', 'xvid'
        0x44495856 => return "XviD".to_string(),                // 'VXID' (alternative)

        // MJPEG
        0x47504A4D | 0x67706a6d => return "MJPEG".to_string(),

        // Catch remaining video codecs before audio
        _ => {}
    }

    // Audio codecs - need to check format tags AND FourCC patterns
    // Common audio format tags (as data1 when lower values)
    if is_standard_mf_format || fourcc <= 0xFFFF {
        match fourcc {
            // Format tags (standard Windows audio format identifiers)
            0x0001 => return "PCM".to_string(),
            0x0003 => return "IEEE Float".to_string(),
            0x0006 => return "A-Law".to_string(),
            0x0007 => return "μ-Law".to_string(),
            0x0055 => return "MP3".to_string(),
            0x00FF => return "AAC".to_string(),
            0x0160 => return "WMA v1".to_string(),
            0x0161 => return "WMA v2".to_string(),
            0x0162 => return "WMA Pro".to_string(),
            0x0163 => return "WMA Lossless".to_string(),
            0x1610 => return "AAC-LC".to_string(),
            0x1612 => return "AAC-HE".to_string(),
            0xA106 => return "AAC (ADTS)".to_string(),
            0xA109 => return "AAC (MPS)".to_string(),
            0x2000 => return "AC-3".to_string(),
            0x2001 => return "DTS".to_string(),
            _ => {}
        }
    }

    // FourCC-based audio codecs
    match fourcc {
        // AAC variants (FourCC encoding)
        0x6134706D => return "AAC".to_string(), // 'mp4a'
        0x61346D70 => return "AAC".to_string(), // 'pm4a' (reversed)
        0x4D344120 => return "AAC".to_string(), // 'M4A '
        0x63614120 => return "AAC".to_string(), // 'cAA ' (rare)

        // Opus (WebM/Matroska audio)
        0x7375704F => return "Opus".to_string(), // 'Opus'
        0x5355504F => return "Opus".to_string(), // 'OPUS'
        0x73757075 => return "Opus".to_string(), // 'upus' (reversed)

        // Vorbis (WebM/OGG audio)
        0x73696272 => return "Vorbis".to_string(), // 'vorbis' partial
        0x62726F56 => return "Vorbis".to_string(), // 'Vorb'
        0x5642524F => return "Vorbis".to_string(), // 'OVRB' (reversed)
        0x7669726F => return "Vorbis".to_string(), // 'oriv'

        // FLAC
        0x43414C46 => return "FLAC".to_string(), // 'FLAC'
        0x63616C66 => return "FLAC".to_string(), // 'flac'

        // AC-3 / E-AC-3
        0x43412D33 => return "AC-3".to_string(),   // 'ac-3'
        0x33432D41 => return "AC-3".to_string(),   // 'A-C3'
        0x43454133 => return "E-AC-3".to_string(), // '3AEC'
        0x332D4345 => return "E-AC-3".to_string(), // 'EC-3'

        // DTS variants
        0x5344544D => return "DTS".to_string(),    // 'DTS '
        0x20535444 => return "DTS".to_string(),    // ' STD'
        0x53544448 => return "DTS-HD".to_string(), // 'HDTS'

        // TrueHD
        0x44484C4D => return "TrueHD".to_string(), // 'MLHD'

        _ => {}
    }

    // Special handling for GUIDs that don't fit the FourCC pattern
    // Check full GUID for some special cases
    let guid_upper = format!(
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

    // Check against known problematic GUIDs
    if guid_upper.starts_with("{6D703461") {
        return "AAC".to_string(); // mp4a in any form
    }
    if guid_upper.starts_with("{7375704F") {
        return "Opus".to_string(); // Opus in any form
    }
    if guid_upper.starts_with("{30395056") || guid_upper.starts_with("{39507600") {
        return "VP9".to_string();
    }

    // If not a known FourCC, try to decode it as ASCII characters
    let bytes = fourcc.to_le_bytes();
    if bytes.iter().all(|&b| b.is_ascii_graphic() || b == b' ') {
        let s: String = bytes.iter().map(|&b| b as char).collect();
        let trimmed = s.trim();
        if !trimmed.is_empty() && trimmed.len() >= 3 {
            // Map common FourCC strings to friendly names
            match trimmed.to_uppercase().as_str() {
                "MP4A" | "AAC " => return "AAC".to_string(),
                "OPUS" => return "Opus".to_string(),
                "FLAC" => return "FLAC".to_string(),
                "VORB" | "VORBIS" => return "Vorbis".to_string(),
                "VP90" | "VP9 " => return "VP9".to_string(),
                "VP80" | "VP8 " => return "VP8".to_string(),
                "AV01" | "AV1 " => return "AV1".to_string(),
                "H264" | "AVC1" => return "H.264/AVC".to_string(),
                "HEVC" | "HVC1" | "H265" => return "H.265/HEVC".to_string(),
                "AC-3" | "AC3 " => return "AC-3".to_string(),
                "EAC3" | "EC-3" => return "E-AC-3".to_string(),
                "DTS " | "DTSH" => return "DTS".to_string(),
                _ => return trimmed.to_string(),
            }
        }
    }

    // Fallback: return full GUID as string (for debugging/unrecognized codecs)
    guid_upper
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guid_to_codec_h264() {
        let guid = windows::core::GUID::from_u128(0x31435641_0000_0010_8000_00AA00389B71);
        assert_eq!(guid_to_codec_name(&guid), "H.264/AVC");
    }

    #[test]
    fn test_guid_to_codec_div3_little_endian() {
        // DIV3 little-endian: 0x33564944
        let guid = windows::core::GUID::from_u128(0x33564944_0000_0010_8000_00AA00389B71);
        assert_eq!(guid_to_codec_name(&guid), "DivX 3");
    }

    #[test]
    fn test_guid_to_codec_div3_big_endian() {
        // VID3 big-endian: 0x33444956
        let guid = windows::core::GUID::from_u128(0x33444956_0000_0010_8000_00AA00389B71);
        assert_eq!(guid_to_codec_name(&guid), "DivX 3");
    }
}
