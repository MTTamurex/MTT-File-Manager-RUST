use super::video_sniffing::DetectionConfidence;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Result of an audio codec sniffing operation
#[derive(Debug, Clone, PartialEq)]
pub struct AudioCodecGuess {
    pub codec: AudioCodec,
    pub confidence: DetectionConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioCodec {
    AAC,
    MP3,
    FLAC,
    Opus,
    Vorbis,
    AC3,
    EAC3,
    ALAC,
    PCM,
    WMA,
    DTS,
    Unknown,
}

impl AudioCodec {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AAC => "AAC",
            Self::MP3 => "MP3",
            Self::FLAC => "FLAC",
            Self::Opus => "Opus",
            Self::Vorbis => "Vorbis",
            Self::AC3 => "AC-3",
            Self::EAC3 => "E-AC-3",
            Self::ALAC => "ALAC",
            Self::PCM => "PCM",
            Self::WMA => "WMA",
            Self::DTS => "DTS",
            Self::Unknown => "Unknown",
        }
    }
}

/// Deterministic audio codec sniffer (Final Fallback)
/// Reads up to 128KB to identify codec via container or bitstream headers.
pub fn sniff_audio_codec(path: &Path) -> Option<AudioCodecGuess> {
    // Skip cloud-only OneDrive files — File::open can trigger download and block
    if crate::infrastructure::onedrive::is_onedrive_path(path)
        && !crate::infrastructure::onedrive::is_locally_available(path)
    {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mut buffer = vec![0u8; 128 * 1024]; // 128 KB
    let bytes_read = file.read(&mut buffer).ok()?;
    if bytes_read < 16 {
        return None;
    }
    let data = &buffer[..bytes_read];

    // 1. Check Container first (Fast & High Confidence)
    if let Some(guess) = sniff_mp4_audio_container(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_mkv_audio_container(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_wav_container(data) {
        return Some(guess);
    }

    // 2. Check Bitstream Signatures (Definitive)
    if let Some(guess) = sniff_flac_signature(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_opus_signature(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_vorbis_signature(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_aac_adts_syncword(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_mp3_syncword(data) {
        return Some(guess);
    }
    if let Some(guess) = sniff_ac3_syncword(data) {
        return Some(guess);
    }

    None
}

/// Sniff MP4/M4A audio structures
fn sniff_mp4_audio_container(data: &[u8]) -> Option<AudioCodecGuess> {
    if data.len() < 8 || &data[4..8] != b"ftyp" {
        return None;
    }

    let search_targets: &[(&[u8], AudioCodec)] = &[
        (b"mp4a", AudioCodec::AAC),
        (b"ac-3", AudioCodec::AC3),
        (b"ec-3", AudioCodec::EAC3),
        (b"alac", AudioCodec::ALAC),
        (b"Opus", AudioCodec::Opus),
    ];

    for (atom, codec) in search_targets {
        if find_byte_pattern(data, atom).is_some() {
            return Some(AudioCodecGuess {
                codec: *codec,
                confidence: DetectionConfidence::High,
            });
        }
    }
    None
}

/// Sniff MKV audio track headers
fn sniff_mkv_audio_container(data: &[u8]) -> Option<AudioCodecGuess> {
    if data.len() < 4 || data[0..4] != [0x1A, 0x45, 0xDF, 0xA3] {
        return None;
    }

    let search_targets: &[(&[u8], AudioCodec)] = &[
        (b"A_AAC", AudioCodec::AAC),
        (b"A_OPUS", AudioCodec::Opus),
        (b"A_AC3", AudioCodec::AC3),
        (b"A_EAC3", AudioCodec::EAC3),
        (b"A_FLAC", AudioCodec::FLAC),
        (b"A_MPEG/L3", AudioCodec::MP3),
        (b"A_VORBIS", AudioCodec::Vorbis),
        (b"A_DTS", AudioCodec::DTS),
    ];

    for (pattern, codec) in search_targets {
        if find_byte_pattern(data, pattern).is_some() {
            return Some(AudioCodecGuess {
                codec: *codec,
                confidence: DetectionConfidence::High,
            });
        }
    }
    None
}

/// Sniff WAV (RIFF) container
fn sniff_wav_container(data: &[u8]) -> Option<AudioCodecGuess> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }

    // Look for 'fmt ' chunk
    if let Some(pos) = find_byte_pattern(data, b"fmt ") {
        if data.len() >= pos + 10 {
            let format_tag = u16::from_le_bytes([data[pos + 8], data[pos + 9]]);
            let codec = match format_tag {
                1 => AudioCodec::PCM,
                0x0055 => AudioCodec::MP3,
                0x00FF => AudioCodec::AAC,
                0x2000 => AudioCodec::AC3,
                0x2001 => AudioCodec::DTS,
                0xFFFE => {
                    // Extensible format, usually PCM
                    AudioCodec::PCM
                }
                _ => return None,
            };
            return Some(AudioCodecGuess {
                codec,
                confidence: DetectionConfidence::Definitive,
            });
        }
    }
    None
}

/// FLAC signature "fLaC"
fn sniff_flac_signature(data: &[u8]) -> Option<AudioCodecGuess> {
    if find_byte_pattern(data, b"fLaC").is_some() {
        return Some(AudioCodecGuess {
            codec: AudioCodec::FLAC,
            confidence: DetectionConfidence::Definitive,
        });
    }
    None
}

/// Opus signature "OpusHead"
fn sniff_opus_signature(data: &[u8]) -> Option<AudioCodecGuess> {
    if find_byte_pattern(data, b"OpusHead").is_some() {
        return Some(AudioCodecGuess {
            codec: AudioCodec::Opus,
            confidence: DetectionConfidence::Definitive,
        });
    }
    None
}

/// Vorbis signature "\x01vorbis"
fn sniff_vorbis_signature(data: &[u8]) -> Option<AudioCodecGuess> {
    if find_byte_pattern(data, b"\x01vorbis").is_some() {
        return Some(AudioCodecGuess {
            codec: AudioCodec::Vorbis,
            confidence: DetectionConfidence::Definitive,
        });
    }
    None
}

/// AAC ADTS Syncword (0xFFF)
fn sniff_aac_adts_syncword(data: &[u8]) -> Option<AudioCodecGuess> {
    for i in 0..data.len() - 2 {
        if data[i] == 0xFF && (data[i + 1] & 0xF0) == 0xF0 {
            // Check potential ADTS header (12 bits syncword)
            // Extra check: Layer is usually 00
            if (data[i + 1] & 0x06) == 0x00 {
                return Some(AudioCodecGuess {
                    codec: AudioCodec::AAC,
                    confidence: DetectionConfidence::High,
                });
            }
        }
    }
    None
}

/// MP3 Frame Sync (0x7FF)
fn sniff_mp3_syncword(data: &[u8]) -> Option<AudioCodecGuess> {
    // Search for 11 bits set at frame start
    for i in 0..data.len() - 2 {
        if data[i] == 0xFF && (data[i + 1] & 0xE0) == 0xE0 {
            // Potential MP3 sync. Check version and layer bits.
            // Version (data[i+1] >> 3) & 0x03 -> 2 (MPEG v2), 3 (MPEG v1)
            // Layer (data[i+1] >> 1) & 0x03 -> 1 (Layer III), 2 (Layer II), 3 (Layer I)
            let version = (data[i + 1] >> 3) & 0x03;
            let layer = (data[i + 1] >> 1) & 0x03;
            if version >= 2 && layer >= 1 {
                return Some(AudioCodecGuess {
                    codec: AudioCodec::MP3,
                    confidence: DetectionConfidence::High,
                });
            }
        }
    }
    None
}

/// AC-3 Syncword (0x0B77)
fn sniff_ac3_syncword(data: &[u8]) -> Option<AudioCodecGuess> {
    if let Some(pos) = find_byte_pattern(data, &[0x0B, 0x77]) {
        if data.len() >= pos + 6 {
            // Bitstream ID is in byte 5 (bits 3-7)
            let bsid = data[pos + 5] >> 3;
            let codec = if bsid > 10 {
                AudioCodec::EAC3
            } else {
                AudioCodec::AC3
            };
            return Some(AudioCodecGuess {
                codec,
                confidence: DetectionConfidence::Definitive,
            });
        }
    }
    None
}

/// Helper to find a byte pattern in data
fn find_byte_pattern(data: &[u8], pattern: &[u8]) -> Option<usize> {
    if pattern.is_empty() {
        return None;
    }
    data.windows(pattern.len())
        .position(|window| window == pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flac_detection() {
        let data = b"some junk fLaC more junk";
        let guess = sniff_flac_signature(data);
        assert!(guess.is_some());
        assert_eq!(guess.unwrap().codec, AudioCodec::FLAC);
    }

    #[test]
    fn test_aac_adts_detection() {
        let data = [0, 1, 2, 0xFF, 0xF1, 0x50, 0x80];
        let guess = sniff_aac_adts_syncword(&data);
        assert!(guess.is_some());
        assert_eq!(guess.unwrap().codec, AudioCodec::AAC);
    }

    #[test]
    fn test_mp3_sync_detection() {
        let data = [0, 0, 0xFF, 0xFB, 0x90, 0x44]; // MPEG v1 Layer III
        let guess = sniff_mp3_syncword(&data);
        assert!(guess.is_some());
        assert_eq!(guess.unwrap().codec, AudioCodec::MP3);
    }
}
