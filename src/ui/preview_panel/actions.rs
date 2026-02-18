use std::path::PathBuf;

pub enum PreviewPanelAction {
    RefreshThumbnail(PathBuf),
    LoadFolderPreview(PathBuf),
    CalculateFolderSize(PathBuf),
    RequestPlay(PathBuf),
    VolumeChanged(f32),
}

pub const PREVIEW_MAX_HEIGHT: f32 = 240.0;
