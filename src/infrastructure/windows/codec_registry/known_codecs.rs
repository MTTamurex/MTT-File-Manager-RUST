use windows::core::GUID;

pub(super) fn check_known_codec(guid: &GUID) -> Option<String> {
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
