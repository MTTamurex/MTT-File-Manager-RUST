use std::path::Path;
use super::MediaMetadata;
use super::property_keys::*;
use super::utils::*;

pub fn is_video_extension(ext: &str) -> bool {
    // Use Windows Perceived Type API for dynamic detection
    crate::infrastructure::windows::file_type::is_video_extension(ext)
}

pub fn read_video_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    // Try Property Store first (fast, uses Windows cache)
    let ps_result = read_video_via_property_store(path);

    let (mut ps_meta_opt, ps_err_opt) = match ps_result {
        Ok(meta) => (Some(meta), None),
        Err(e) => (None, Some(e)),
    };

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
        if let Some(mf_meta) = crate::infrastructure::windows::media_foundation::extract_video_metadata_mf(path) {
            let base = ps_meta_opt.take().unwrap_or_default();
            return Ok(merge_video_metadata(base, mf_meta, path));
        }
    }

    let mut final_meta = if let Some(meta) = ps_meta_opt {
        meta
    } else if let Some(ps_err) = ps_err_opt {
        return Err(ps_err);
    } else {
        MediaMetadata::default()
    };

    // Final Fallback: Bitstream Sniffing (if codec is unknown or cryptic)
    let is_cryptic = |c: &Option<String>| {
        if let Some(s) = c {
            s.len() == 8 && s.chars().all(|ch| ch.is_ascii_hexdigit())
        } else {
            true
        }
    };

    if is_cryptic(&final_meta.video_codec) {
        if let Some(guess) = super::video_sniffing::sniff_video_codec(path) {
            final_meta.video_codec = Some(format!("{} (Sniffed)", guess.codec.as_str()));
        }
    }

    if is_cryptic(&final_meta.audio_codec) {
        if let Some(guess) = super::audio_sniffing::sniff_audio_codec(path) {
            final_meta.audio_codec = Some(format!("{} (Sniffed)", guess.codec.as_str()));
        }
    }

    Ok(final_meta)
}

pub fn read_video_via_property_store(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
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

    let fourcc = unsafe { read_fourcc(&store, &PKEY_VIDEO_FOURCC) };
    let stream_name = unsafe { read_string(&store, &PKEY_VIDEO_STREAMNAME) };
    let stream_description = unsafe { read_string(&store, &PKEY_VIDEO_STREAMDESCRIPTION) };
    let subtitle = unsafe { read_string(&store, &PKEY_MEDIA_SUBTITLE) };
    let encoding_settings = unsafe { read_string(&store, &PKEY_MEDIA_ENCODINGSETTINGS) };
    let content_type = unsafe { read_string(&store, &PKEY_MEDIA_CONTENTTYPE) };
    let compression = unsafe { read_string(&store, &PKEY_VIDEO_COMPRESSION) };

    let ogm_video = unsafe { read_string(&store, &PKEY_VIDEO_TRACKS) };
    let ogm_audio = unsafe { read_string(&store, &PKEY_AUDIO_TRACKS) };

    let video_codec = stream_description
        .as_ref()
        .and_then(|d| detect_codec_from_description(d))
        .or_else(|| {
            stream_name
                .as_ref()
                .and_then(|s| detect_codec_from_description(s))
        })
        .or_else(|| {
            subtitle
                .as_ref()
                .and_then(|s| detect_codec_from_description(s))
        })
        .or_else(|| fourcc.clone())
        .or_else(|| ogm_video.clone())
        .or_else(|| encoding_settings.clone())
        .or_else(|| content_type.clone().filter(|s| !s.is_empty()))
        .or_else(|| {
            let comp = compression.clone()?;
            let file_ext = path.extension()?.to_str()?.to_uppercase();
            let compression_upper = comp.to_uppercase();
            if compression_upper == file_ext || compression_upper == "VIDEO" {
                None
            } else {
                Some(comp)
            }
        });

    // --- NEW: Robust Fallback for Resolution and FPS from Description ---
    let mut width = width;
    let mut height = height;
    let mut frame_rate = frame_rate;

    if width.is_none() || height.is_none() || frame_rate.is_none() {
        if let Some(desc) = &stream_description {
            let (ext_w, ext_h, ext_fps) = parse_resolution_and_fps_from_description(desc);
            if width.is_none() { width = ext_w; }
            if height.is_none() { height = ext_h; }
            if frame_rate.is_none() { frame_rate = ext_fps; }
        }
    }
    // --------------------------------------------------------------------

    let audio_compression = unsafe { read_string(&store, &PKEY_AUDIO_COMPRESSION) };
    let audio_stream_name = unsafe { read_string(&store, &PKEY_AUDIO_STREAMNAME) };
    let audio_format = unsafe { read_string(&store, &PKEY_AUDIO_FORMAT) };

    let audio_codec = audio_compression
        .or_else(|| audio_stream_name.clone())
        .or_else(|| ogm_audio.clone())
        .or_else(|| audio_format.clone())
        .map(|s| sanitize_codec_string(&s));

    let audio_bitrate = unsafe { read_u32(&store, &PKEY_AUDIO_ENCODINGBITRATE) };
    let audio_channels = unsafe { read_u32(&store, &PKEY_AUDIO_CHANNELCOUNT) };
    let video_bitrate = unsafe { read_u32(&store, &PKEY_VIDEO_ENCODINGBITRATE) };

    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.to_uppercase());

    let video_codec_final = video_codec.or_else(|| {
        let filename = path.file_name()?.to_str()?;
        let filename_lower = filename.to_lowercase();

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

        None
    });

    let video_codec_sanitized = video_codec_final
        .map(|s| sanitize_codec_string(&s))
        .filter(|s| !s.is_empty() && !is_container_name(s, path));

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
        video_codec: video_codec_sanitized,
        audio_codec,
        audio_bitrate,
        audio_channels,
        ..Default::default()
    })
}

pub fn merge_video_metadata(
    ps: MediaMetadata,
    mf: crate::infrastructure::windows::media_foundation::VideoMetadataMF,
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
        video_codec,
        audio_codec,
        audio_bitrate: ps.audio_bitrate.or(mf.audio_bitrate),
        audio_channels: ps.audio_channels.or(mf.audio_channels),
        ..Default::default()
    }
}

pub fn detect_codec_from_description(description: &str) -> Option<String> {
    let desc = description.to_uppercase();

    if desc.contains("H.264") || desc.contains("AVC") || desc.contains("X264") {
        return Some("H.264/AVC".to_string());
    }
    if desc.contains("H.265") || desc.contains("HEVC") || desc.contains("X265") {
        return Some("H.265/HEVC".to_string());
    }
    if desc.contains("VP9") {
        return Some("VP9".to_string());
    }
    if desc.contains("VP8") {
        return Some("VP8".to_string());
    }
    if desc.contains("AV1") || desc.contains("AV01") {
        return Some("AV1".to_string());
    }
    if desc.contains("MPEG-4") {
        return Some("MPEG-4".to_string());
    }
    if desc.contains("XVID") {
        return Some("XviD".to_string());
    }
    if desc.contains("DIVX") || desc.contains("DX50") {
        return Some("DivX".to_string());
    }
    if desc.contains("DIV3") || desc.contains("MP43") {
        return Some("DivX 3".to_string());
    }
    if desc.contains("PRORES") {
        return Some("ProRes".to_string());
    }
    if desc.contains("MJPEG") || desc.contains("MOTION JPEG") {
        return Some("MJPEG".to_string());
    }
    if desc.contains("WMV3") || desc.contains("WMV9") {
        return Some("WMV".to_string());
    }
    if desc.contains("THEORA") {
        return Some("Theora".to_string());
    }
    if desc.contains("VORBIS") {
        return Some("Vorbis".to_string());
    }
    if desc.contains("FLAC") {
        return Some("FLAC".to_string());
    }
    if desc.contains("MPEG-2") || desc.contains("MPEG2") {
        return Some("MPEG-2".to_string());
    }

    None
}

pub fn sanitize_codec_string(s: &str) -> String {
    let s = s.trim();

    if s.starts_with('{') && s.contains('-') {
        return crate::infrastructure::windows::codec_registry::resolve_codec_guid(s);
    }

    if s.len() == 8 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return crate::infrastructure::windows::codec_registry::resolve_codec_guid(s);
    }

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
    if upper == "MP43" || upper == "DIV3" {
        return "DivX 3".to_string();
    }
    if upper.contains("XVID") {
        return "XviD".to_string();
    }

    s.to_string()
}

pub fn is_container_name(codec: &str, path: &Path) -> bool {
    let codec_lower = codec.to_lowercase();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if codec_lower == ext {
        return true;
    }

    matches!(
        codec_lower.as_str(),
        "mkv"
            | "webm"
            | "mp4"
            | "avi"
            | "mov"
            | "wmv"
            | "flv"
            | "ogm"
            | "ogg"
            | "video"
            | "audio"
            | "matroska"
            | "container"
    )
}

/// Helper to parse resolution (e.g., \"1920x1080\") and FPS (e.g., \"23.97 fps\") from a description string.
fn parse_resolution_and_fps_from_description(desc: &str) -> (Option<u32>, Option<u32>, Option<f32>) {
    let mut width = None;
    let mut height = None;
    let mut fps = None;

    // Use regex-free parsing for performance
    let parts: Vec<&str> = desc.split(|c: char| !c.is_ascii_alphanumeric() && c != '.').filter(|s| !s.is_empty()).collect();

    for i in 0..parts.len() {
        let p = parts[i].to_uppercase();
        
        // Look for 1920x1080 pattern
        if p.contains('X') {
            let dim_parts: Vec<&str> = p.split('X').collect();
            if dim_parts.len() == 2 {
                let w = dim_parts[0].parse::<u32>().ok();
                let h = dim_parts[1].parse::<u32>().ok();
                if w.is_some() && h.is_some() {
                    width = w;
                    height = h;
                }
            }
        }

        // Look for numbers followed by \"FPS\"
        if p == "FPS" && i > 0 {
            if let Ok(val) = parts[i-1].parse::<f32>() {
                fps = Some(val);
            }
        }
    }

    (width, height, fps)
}
