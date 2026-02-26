use std::path::PathBuf;

pub enum PreviewPanelAction {
    RefreshThumbnail(PathBuf),
    LoadFolderPreview(PathBuf),
    CalculateFolderSize(PathBuf),
    RequestPlay(PathBuf),
    VolumeChanged(f32),
    /// Detach the current video to a standalone OS window.
    /// Carries (path, current_position, current_volume).
    DetachVideo {
        path: PathBuf,
        position: f64,
        volume: f32,
    },
}

pub const PREVIEW_MAX_HEIGHT: f32 = 240.0;
