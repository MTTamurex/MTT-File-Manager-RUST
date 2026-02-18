use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::ui::theme;

pub(super) struct StartupPreferences {
    pub(super) sort_mode: SortMode,
    pub(super) sort_mode_computer: SortMode,
    pub(super) sort_mode_normal: SortMode,
    pub(super) sort_descending: bool,
    pub(super) folders_position: FoldersPosition,
    pub(super) thumbnail_size: f32,
    pub(super) view_mode: ViewMode,
    pub(super) show_preview_panel: bool,
    pub(super) upload_budget_ms: f32,
    pub(super) saved_window_width: f32,
    pub(super) saved_window_height: f32,
    pub(super) saved_is_maximized: bool,
    pub(super) sidebar_left_width: f32,
    pub(super) sidebar_right_width: f32,
    pub(super) session_volume: f32,
}

impl StartupPreferences {
    pub(super) fn load(disk_cache: &ThumbnailDiskCache) -> Self {
        let sort_mode = disk_cache
            .get_preference("sort_mode")
            .map(|s| match s.as_str() {
                "date" => SortMode::Date,
                "size" => SortMode::Size,
                "type" => SortMode::Type,
                "drive_total" => SortMode::DriveTotalSpace,
                "drive_free" => SortMode::DriveFreeSpace,
                _ => SortMode::Name,
            })
            .unwrap_or(SortMode::Name);

        let sort_mode_computer = disk_cache
            .get_preference("sort_mode_computer")
            .map(|s| match s.as_str() {
                "drive_total" => SortMode::DriveTotalSpace,
                "drive_free" => SortMode::DriveFreeSpace,
                _ => SortMode::Name,
            })
            .unwrap_or(SortMode::Name);

        let sort_mode_normal = disk_cache
            .get_preference("sort_mode_normal")
            .map(|s| match s.as_str() {
                "date" => SortMode::Date,
                "size" => SortMode::Size,
                "type" => SortMode::Type,
                _ => SortMode::Name,
            })
            .unwrap_or(SortMode::Name);

        let sort_descending = disk_cache
            .get_preference("sort_descending")
            .map(|s| s == "true")
            .unwrap_or(false);

        let folders_position = disk_cache
            .get_preference("folders_position")
            .map(|s| match s.as_str() {
                "last" => FoldersPosition::Last,
                "mixed" => FoldersPosition::Mixed,
                _ => FoldersPosition::First,
            })
            .unwrap_or(FoldersPosition::First);

        let thumbnail_size = disk_cache
            .get_preference("thumbnail_size")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(theme::THUMBNAIL_DEFAULT)
            .clamp(theme::THUMBNAIL_MIN, theme::THUMBNAIL_MAX);

        let view_mode = disk_cache
            .get_preference("view_mode")
            .map(|s| match s.as_str() {
                "list" => ViewMode::List,
                _ => ViewMode::Grid,
            })
            .unwrap_or(ViewMode::Grid);

        let show_preview_panel = disk_cache
            .get_preference("show_preview_panel")
            .map(|s| s != "false")
            .unwrap_or(true);

        let upload_budget_ms = disk_cache
            .get_preference("upload_budget_ms")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(6.0)
            .clamp(2.0, 10.0);

        let saved_window_width = disk_cache
            .get_preference("window_width")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1280.0);
        let saved_window_height = disk_cache
            .get_preference("window_height")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(720.0);
        let saved_is_maximized = disk_cache
            .get_preference("window_is_maximized")
            .map(|s| s == "true")
            .unwrap_or(true);

        let sidebar_left_raw = disk_cache.get_preference("sidebar_left_width");
        let sidebar_right_raw = disk_cache.get_preference("sidebar_right_width");

        log::debug!(
            "[INIT] Raw sidebar values from DB: L={:?}, R={:?}",
            sidebar_left_raw,
            sidebar_right_raw
        );

        let sidebar_left_width = sidebar_left_raw
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(200.0);
        let sidebar_right_width = sidebar_right_raw
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(300.0);

        log::debug!(
            "[INIT] Parsed sidebar widths: L={}, R={}",
            sidebar_left_width,
            sidebar_right_width
        );

        let session_volume = disk_cache
            .get_preference("media_volume")
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        Self {
            sort_mode,
            sort_mode_computer,
            sort_mode_normal,
            sort_descending,
            folders_position,
            thumbnail_size,
            view_mode,
            show_preview_panel,
            upload_budget_ms,
            saved_window_width,
            saved_window_height,
            saved_is_maximized,
            sidebar_left_width,
            sidebar_right_width,
            session_volume,
        }
    }
}
