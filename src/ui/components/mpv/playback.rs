use super::state::{MpvState, TrackInfo};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

const EXTERNAL_SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "vtt", "sub"];

/// Play commands wrapper
pub fn play(mpv: &Option<Arc<mpv::Mpv>>) {
    if let Some(m) = mpv {
        let _ = m.set_property("pause", false);
    }
}

/// Pause command wrapper
pub fn pause(mpv: &Option<Arc<mpv::Mpv>>) {
    if let Some(m) = mpv {
        let _ = m.set_property("pause", true);
    }
}

/// Seek to absolute time
pub fn seek(mpv: &Option<Arc<mpv::Mpv>>, time: f64) {
    if let Some(m) = mpv {
        let _ = m.set_property("time-pos", time.max(0.0));
    }
}

/// Seek relative to current position
pub fn seek_relative(mpv: &Option<Arc<mpv::Mpv>>, delta_seconds: f64) {
    if let Some(m) = mpv {
        if let Ok(current) = m.get_property::<f64>("time-pos") {
            if let Ok(duration) = m.get_property::<f64>("duration") {
                let new_time = (current + delta_seconds).clamp(0.0, duration);
                let _ = m.set_property("time-pos", new_time);
            }
        }
    }
}

/// Set volume (0.0 to 1.0)
pub fn set_volume(mpv: &Option<Arc<mpv::Mpv>>, state: &Arc<RwLock<MpvState>>, volume: f32) {
    let clamped = volume.clamp(0.0, 1.0);
    if let Some(m) = mpv {
        let _ = m.set_property("volume", (clamped * 100.0) as f64);
        let _ = m.set_property("mute", false);
    }
    if let Ok(mut s) = state.write() {
        s.volume = clamped;
        s.is_muted = false;
    }
}

/// Set mute state
pub fn set_muted(mpv: &Option<Arc<mpv::Mpv>>, state: &Arc<RwLock<MpvState>>, muted: bool) {
    if let Some(m) = mpv {
        let _ = m.set_property("mute", muted);
    }
    if let Ok(mut s) = state.try_write() {
        s.is_muted = muted;
    }
}

/// Set audio track by ID
pub fn set_audio_track(
    mpv: &Option<Arc<mpv::Mpv>>,
    state: &Arc<RwLock<MpvState>>,
    cached_tracks: &mut Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    id: i64,
) {
    if let Some(m) = mpv {
        let _ = m.set_property("aid", id);
    }
    // Update local state to reflect selection
    if let Ok(mut s) = state.write() {
        for track in &mut s.audio_tracks {
            track.selected = track.id == id;
        }
    }
    // Invalidate cache so it will be refreshed
    if let Some((ref mut audio, _)) = cached_tracks {
        for track in audio {
            track.selected = track.id == id;
        }
    }
}

/// Set subtitle track by ID
pub fn set_subtitle_track(
    mpv: &Option<Arc<mpv::Mpv>>,
    state: &Arc<RwLock<MpvState>>,
    cached_tracks: &mut Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    id: i64,
) {
    if let Some(m) = mpv {
        let _ = m.set_property("sid", id);
    }
    // Update local state to reflect selection (id=0 means disabled)
    if let Ok(mut s) = state.write() {
        for track in &mut s.subtitle_tracks {
            track.selected = track.id == id;
        }
    }
    // Update cache
    if let Some((_, ref mut subs)) = cached_tracks {
        for track in subs {
            track.selected = track.id == id;
        }
    }
}

/// Find sidecar subtitle near the video file using strict basename match.
///
/// Rule:
/// - Only files with EXACT same basename as the video are accepted
///   (movie.mkv -> movie.srt/movie.ass/etc.)
pub fn find_sidecar_subtitle(video_path: &Path) -> Option<PathBuf> {
    let parent = video_path.parent()?;
    let stem = video_path.file_stem()?.to_str()?;

    // Strict basename match with extension priority order.
    for ext in EXTERNAL_SUBTITLE_EXTENSIONS {
        let candidate = parent.join(format!("{}.{}", stem, ext));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

/// Add external subtitle and select it immediately.
pub fn load_external_subtitle(
    mpv: &Option<Arc<mpv::Mpv>>,
    state: &Arc<RwLock<MpvState>>,
    cached_tracks: &mut Option<(Vec<TrackInfo>, Vec<TrackInfo>)>,
    subtitle_path: &Path,
) -> Result<(), String> {
    if !subtitle_path.is_file() {
        return Err(rust_i18n::t!("video.subtitle_not_found", path = subtitle_path.display().to_string()).to_string());
    }

    let m = mpv
        .as_ref()
        .ok_or_else(|| rust_i18n::t!("video.player_not_init").to_string())?;

    let subtitle_str = subtitle_path.to_string_lossy().to_string();
    m.command("sub-add", &[&subtitle_str, "select"])
        .map_err(|e| rust_i18n::t!("video.subtitle_load_failed", error = format!("{:?}", e)).to_string())?;

    // Force refresh of subtitle track list after sub-add.
    *cached_tracks = None;

    if let Ok(mut s) = state.write() {
        for track in &mut s.subtitle_tracks {
            track.selected = false;
        }
    }

    Ok(())
}

/// Check if file is ready by checking if duration is available
pub fn is_file_ready(mpv: &Arc<mpv::Mpv>) -> bool {
    mpv.get_property::<f64>("duration")
        .map(|d| d > 0.0)
        .unwrap_or(false)
}

/// Detect interlaced status from MPV video params.
/// Returns None if detection is inconclusive.
pub fn detect_interlaced(mpv: &mpv::Mpv) -> Option<bool> {
    if let Ok(value) = mpv.get_property::<bool>("video-params/interlaced") {
        return Some(value);
    }
    if let Ok(value) = mpv.get_property::<i64>("video-params/interlaced") {
        return Some(value != 0);
    }
    if let Ok(value) = mpv.get_property::<String>("video-params/interlaced") {
        let value = value.to_lowercase();
        if value == "yes" || value == "true" || value == "1" {
            return Some(true);
        }
        if value == "no" || value == "false" || value == "0" {
            return Some(false);
        }
    }
    if let Ok(field) = mpv.get_property::<String>("video-params/field") {
        let field = field.to_lowercase();
        if field == "top" || field == "bottom" || field == "tff" || field == "bff" {
            return Some(true);
        }
        if field == "progressive" {
            return Some(false);
        }
    }
    None
}

/// Get current video aspect ratio from MPV.
/// Prefers post-VO params and falls back to source params.
pub fn get_video_aspect(mpv: &mpv::Mpv) -> Option<f64> {
    if let Ok(aspect) = mpv.get_property::<f64>("video-out-params/aspect") {
        if aspect.is_finite() && aspect > 0.1 {
            return Some(aspect);
        }
    }
    let dw = mpv.get_property::<i64>("video-out-params/dw").ok().unwrap_or(0);
    let dh = mpv.get_property::<i64>("video-out-params/dh").ok().unwrap_or(0);
    if dw > 0 && dh > 0 {
        return Some(dw as f64 / dh as f64);
    }
    let w = mpv.get_property::<i64>("video-params/w").ok().unwrap_or(0);
    let h = mpv.get_property::<i64>("video-params/h").ok().unwrap_or(0);
    if w > 0 && h > 0 {
        return Some(w as f64 / h as f64);
    }
    None
}

/// Query tracks from MPV and return audio and subtitle tracks
pub fn query_tracks(mpv: &Arc<mpv::Mpv>) -> (Vec<TrackInfo>, Vec<TrackInfo>) {
    let mut audio_tracks = Vec::new();
    let mut sub_tracks = Vec::new();

    if let Ok(count) = mpv.get_property::<i64>("track-list/count") {
        if count > 0 {
            for i in 0..count {
                let base = format!("track-list/{}/", i);
                let t_type = mpv
                    .get_property::<String>(&(base.clone() + "type"))
                    .unwrap_or_default();
                let id = mpv.get_property::<i64>(&(base.clone() + "id")).unwrap_or(0);
                let selected = mpv
                    .get_property::<bool>(&(base.clone() + "selected"))
                    .unwrap_or(false);
                let title = mpv.get_property::<String>(&(base.clone() + "title")).ok();
                let lang = mpv.get_property::<String>(&(base + "lang")).ok();

                let info = TrackInfo {
                    id,
                    track_type: t_type.clone(),
                    title,
                    lang,
                    selected,
                };

                if t_type == "audio" {
                    audio_tracks.push(info);
                } else if t_type == "sub" {
                    sub_tracks.push(info);
                }
            }
        }
    }

    (audio_tracks, sub_tracks)
}
