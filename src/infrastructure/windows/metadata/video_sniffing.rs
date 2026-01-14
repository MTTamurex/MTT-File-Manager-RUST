use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Result of a codec sniffing operation
#[derive(Debug, Clone, PartialEq)]
pub struct CodecGuess {
    pub codec: VideoCodec,
    pub confidence: DetectionConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VideoCodec {
    H264,
    HEVC,
    AV1,
    VP9,
    MPEG2,
    MPEG4,
    Unknown,
}

impl VideoCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::H264 => "H.264/AVC",
            Self::HEVC => "H.265/HEVC",
            Self::AV1 => "AV1",
            Self::VP9 => "VP9",
            Self::MPEG2 => "MPEG-2",
            Self::MPEG4 => "MPEG-4",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum DetectionConfidence {
    Low = 1,
    Medium = 2,
    High = 3,
    Definitive = 4,
}

/// Deterministic video codec sniffer (Final Fallback)
/// Reads up to 256KB to identify codec via container or bitstream headers.
pub fn sniff_video_codec(path: &Path) -> Option<CodecGuess> {
    let mut file = File::open(path).ok()?;
    let mut buffer = vec![0u8; 256 * 1024]; // 256 KB
    let bytes_read = file.read(&mut buffer).ok()?;
    if bytes_read < 16 {
        return None;
    }
    let data = &buffer[..bytes_read];

    // 1. Check Container first (Fast & High Confidence)
    if let Some(guess) = sniff_mp4_container(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_mkv_container(data) {
        return Some(guess);
    }

    // 2. Check Bitstream (Definitive)
    if let Some(guess) = sniff_h264_bitstream(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_hevc_bitstream(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_av1_bitstream(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_vp9_bitstream(data) {
        return Some(guess);
    }

    None
}

/// Sniff MP4/MOV atoms (avc1, hvc1, etc.)
fn sniff_mp4_container(data: &[u8]) -> Option<CodecGuess> {
    // Quick check for 'ftyp' atom at start
    if data.len() < 8 || &data[4..8] != b"ftyp" {
        return None;
    }

    // Search for known codec atoms in the first 256KB
    // We look for 4-character codes (FourCC)
    let search_targets: &[(&[u8], VideoCodec)] = &[
        (b"avc1", VideoCodec::H264),
        (b"avc3", VideoCodec::H264),
        (b"hvc1", VideoCodec::HEVC),
        (b"hev1", VideoCodec::HEVC),
        (b"av01", VideoCodec::AV1),
        (b"vp09", VideoCodec::VP9),
    ];

    for (atom, codec) in search_targets {
        if find_byte_pattern(data, atom).is_some() {
            return Some(CodecGuess {
                codec: *codec,
                confidence: DetectionConfidence::High,
            });
        }
    }

    None
}

/// Sniff MKV/WebM (EBML) headers
fn sniff_mkv_container(data: &[u8]) -> Option<CodecGuess> {
    // MKV starts with EBML ID: [0x1A, 0x45, 0xDF, 0xA3]
    if data.len() < 4 || &data[0..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        return None;
    }

    let search_targets: &[(&[u8], VideoCodec)] = &[
        (b"V_MPEG4/ISO/AVC", VideoCodec::H264),
        (b"V_MPEGH/ISO/HEVC", VideoCodec::HEVC),
        (b"V_AV1", VideoCodec::AV1),
        (b"V_VP9", VideoCodec::VP9),
    ];

    for (pattern, codec) in search_targets {
        if find_byte_pattern(data, pattern).is_some() {
            return Some(CodecGuess {
                codec: *codec,
                confidence: DetectionConfidence::High,
            });
        }
    }

    None
}

/// H.264 Bitstream analysis: Start Code (00 00 01) + NAL Type (7=SPS, 8=PPS)
fn sniff_h264_bitstream(data: &[u8]) -> Option<CodecGuess> {
    // H.264 NAL Unit: 00 00 00 01 (or 00 00 01)
    // NAL Header byte: [forbidden_zero_bit(1) | nal_ref_idc(2) | nal_unit_type(5)]
    // NAL Type 7 = SPS (Sequence Parameter Set)
    // NAL Type 8 = PPS (Picture Parameter Set)

    let mut found_sps = false;
    let mut found_pps = false;

    let mut i = 0;
    while i < data.len() - 5 {
        if data[i] == 0 && data[i + 1] == 0 && (data[i + 2] == 1 || (data[i + 2] == 0 && data[i + 3] == 1)) {
            let offset = if data[i + 2] == 1 { 3 } else { 4 };
            if i + offset >= data.len() { break; }
            
            let nal_type = data[i + offset] & 0x1F;
            if nal_type == 7 { found_sps = true; }
            if nal_type == 8 { found_pps = true; }

            if found_sps && found_pps {
                return Some(CodecGuess {
                    codec: VideoCodec::H264,
                    confidence: DetectionConfidence::Definitive,
                });
            }
            i += offset;
        }
        i += 1;
    }

    // Also check for AVCC configuration record (avcC)
    if find_byte_pattern(data, b"avcC").is_some() {
        return Some(CodecGuess {
            codec: VideoCodec::H264,
            confidence: DetectionConfidence::High,
        });
    }

    None
}

/// HEVC (H.265) Bitstream analysis: NAL Types (32=VPS, 33=SPS, 34=PPS)
fn sniff_hevc_bitstream(data: &[u8]) -> Option<CodecGuess> {
    // HEVC NAL Header is 2 bytes:
    // [forbidden_zero_bit(1) | nal_unit_type(6) | nuh_layer_id(6) | nuh_temporal_id_plus1(3)]
    // NAL Type 32 = VPS, 33 = SPS, 34 = PPS

    let mut found_vps = false;
    let mut found_sps = false;

    let mut i = 0;
    while i < data.len() - 6 {
        if data[i] == 0 && data[i + 1] == 0 && (data[i + 2] == 1 || (data[i + 2] == 0 && data[i + 3] == 1)) {
            let offset = if data[i + 2] == 1 { 3 } else { 4 };
            if i + offset >= data.len() { break; }

            let nal_type = (data[i + offset] >> 1) & 0x3F;
            if nal_type == 32 { found_vps = true; }
            if nal_type == 33 { found_sps = true; }

            if found_vps && found_sps {
                return Some(CodecGuess {
                    codec: VideoCodec::HEVC,
                    confidence: DetectionConfidence::Definitive,
                });
            }
            i += offset;
        }
        i += 1;
    }

    if find_byte_pattern(data, b"hvcC").is_some() {
        return Some(CodecGuess {
            codec: VideoCodec::HEVC,
            confidence: DetectionConfidence::High,
        });
    }

    None
}

/// AV1 Bitstream analysis (OBU Sequence Header)
fn sniff_av1_bitstream(data: &[u8]) -> Option<CodecGuess> {
    // AV1 uses OBU (Open Bitstream Units)
    // Looking for OBU_SEQUENCE_HEADER (type 1)
    // OBU Header: [forbidden_bit(1) | obu_type(4) | obu_extension_flag(1) | obu_has_size_field(1) | reserved_bit(1)]
    
    // Pattern search for AV01 (container) or common bitstream signatures
    if find_byte_pattern(data, b"av01").is_some() {
        return Some(CodecGuess {
            codec: VideoCodec::AV1,
            confidence: DetectionConfidence::High,
        });
    }

    // Pure bitstream detection for AV1 is complex, but we can look for the sequence header pattern
    // Usually starts with a Temporal Delimiter OBU (type 2) or Sequence Header (type 1)
    None
}

/// VP9 Bitstream analysis (Sync code 0x49 0x83 0x42)
fn sniff_vp9_bitstream(data: &[u8]) -> Option<CodecGuess> {
    // VP9 frames start with 0b10 marker
    // And have a sync code for keyframes: 0x49 0x83 0x42
    if find_byte_pattern(data, &[0x49, 0x83, 0x42]).is_some() {
        return Some(CodecGuess {
            codec: VideoCodec::VP9,
            confidence: DetectionConfidence::High,
        });
    }
    None
}

/// Helper to find a byte pattern in data
fn find_byte_pattern(data: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() { return None; }
    data.windows(pattern.len())
        .position(|window| window == pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h264_sps_pps_detection() {
        let mut data = vec![0u8; 100];
        // Fake H.264 stream: [StartCode] [NAL Type 7] ... [StartCode] [NAL Type 8]
        data[10..14].copy_from_slice(&[0, 0, 0, 1]);
        data[14] = 0x67; // Type 7 (SPS)
        data[30..34].copy_from_slice(&[0, 0, 0, 1]);
        data[34] = 0x68; // Type 8 (PPS)

        let guess = sniff_h264_bitstream(&data);
        assert!(guess.is_some());
        assert_eq!(guess.unwrap().codec, VideoCodec::H264);
    }

    #[test]
    fn test_hevc_vps_sps_detection() {
        let mut data = vec![0u8; 100];
        // Fake HEVC stream: [StartCode] [NAL Type 32] ... [StartCode] [NAL Type 33]
        // NAL Type 32 = 0x40 (binary: 0 100000 0000000 0)
        // NAL Type 33 = 0x42 (binary: 0 100001 0000000 0)
        data[10..14].copy_from_slice(&[0, 0, 0, 1]);
        data[14] = 32 << 1; 
        data[30..34].copy_from_slice(&[0, 0, 0, 1]);
        data[34] = 33 << 1;

        let guess = sniff_hevc_bitstream(&data);
        assert!(guess.is_some());
        assert_eq!(guess.unwrap().codec, VideoCodec::HEVC);
    }
}
