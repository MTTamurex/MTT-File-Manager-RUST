use super::property_keys::*;
use super::utils::*;
use super::video::sanitize_codec_string;
use super::MediaMetadata;
use std::path::Path;

pub fn read_audio_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    // Try Property Store first (fast, uses Windows cache)
    let mut meta = read_audio_via_property_store(path)?;

    // Fallback: MediaFoundation for missing fields
    let need_mf = meta.duration_100ns.is_none()
        || meta.audio_codec.is_none()
        || meta.audio_channels.is_none()
        || meta.audio_bitrate.is_none();

    if need_mf {
        if let Some(mf_meta) =
            crate::infrastructure::windows::media_foundation::extract_video_metadata_mf(path)
        {
            if meta.duration_100ns.is_none() {
                meta.duration_100ns = mf_meta.duration_100ns;
            }
            if meta.audio_channels.is_none() {
                meta.audio_channels = mf_meta.audio_channels;
            }
            if meta.audio_bitrate.is_none() {
                meta.audio_bitrate = mf_meta.audio_bitrate;
            }
            if meta.audio_sample_rate.is_none() {
                meta.audio_sample_rate = mf_meta.audio_sample_rate;
            }
            if meta.audio_codec.is_none() {
                meta.audio_codec = mf_meta.audio_codec_guid;
            }
        }
    }

    // Final fallback: codec sniffing
    let is_cryptic = |c: &Option<String>| {
        if let Some(s) = c {
            s.len() == 8 && s.chars().all(|ch| ch.is_ascii_hexdigit())
        } else {
            true
        }
    };

    if is_cryptic(&meta.audio_codec) {
        if let Some(guess) = super::audio_sniffing::sniff_audio_codec(path) {
            meta.audio_codec = Some(guess.codec.as_str().to_string());
        }
    }

    Ok(meta)
}

fn read_audio_via_property_store(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    let _com_guard = ComGuard::new();
    let store = unsafe { open_property_store(path)? };

    let duration_100ns = unsafe { read_u64(&store, &PKEY_MEDIA_DURATION) };
    let audio_bitrate = unsafe { read_u32(&store, &PKEY_AUDIO_ENCODINGBITRATE) };
    let audio_channels = unsafe { read_u32(&store, &PKEY_AUDIO_CHANNELCOUNT) };
    let audio_sample_rate = unsafe { read_u32(&store, &PKEY_AUDIO_SAMPLERATE) };

    let audio_compression = unsafe { read_string(&store, &PKEY_AUDIO_COMPRESSION) };
    let audio_stream_name = unsafe { read_string(&store, &PKEY_AUDIO_STREAMNAME) };
    let audio_format = unsafe { read_string(&store, &PKEY_AUDIO_FORMAT) };

    let audio_codec = audio_compression
        .or_else(|| audio_stream_name.clone())
        .or_else(|| audio_format.clone())
        .map(|s| sanitize_codec_string(&s));

    // Music tags
    let artist = unsafe { read_string(&store, &PKEY_MUSIC_ARTIST) };
    let album = unsafe { read_string(&store, &PKEY_MUSIC_ALBUMTITLE) };
    let track_title = unsafe { read_string(&store, &PKEY_TITLE) };
    let genre = unsafe { read_string(&store, &PKEY_MUSIC_GENRE) };
    let year = unsafe { read_u32(&store, &PKEY_MEDIA_YEAR) };

    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| ext.to_uppercase());

    Ok(MediaMetadata {
        duration_100ns,
        format,
        audio_codec,
        audio_bitrate,
        audio_channels,
        audio_sample_rate,
        artist,
        album,
        track_title,
        genre,
        year,
        ..Default::default()
    })
}
