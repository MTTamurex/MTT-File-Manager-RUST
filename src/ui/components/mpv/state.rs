/// Track information for audio/subtitles.
#[derive(Clone, Debug, Default)]
pub struct TrackInfo {
    pub id: i64,
    pub track_type: String, // "audio", "video", "sub"
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
}

/// Shared state for MPV playback.
#[derive(Clone, Default)]
pub struct MpvState {
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_muted: bool,
    pub audio_tracks: Vec<TrackInfo>,
    pub subtitle_tracks: Vec<TrackInfo>,
}