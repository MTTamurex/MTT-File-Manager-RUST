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
    /// Navigate to the clicked breadcrumb segment (a parent folder on disk).
    NavigateTo(PathBuf),
    /// User requested an on-demand SHA-256 calculation for the selected file.
    CalculateFileHash(PathBuf),
}

pub const PREVIEW_MAX_HEIGHT: f32 = 240.0;
