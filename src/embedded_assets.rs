/// Embedded assets module
/// This module contains all assets embedded at compile time to make the executable portable

// Embed the Remix Icon font
pub const REMIXICON_TTF: &[u8] = include_bytes!("../assets/remixicon.ttf");

// Embed application icon (PNG)
pub const APP_ICON_PNG: &[u8] = include_bytes!("../appicon.png");

// Embed SVG icons
pub const ICON_COPY: &[u8] = include_bytes!("../assets/icons/copy.svg");
pub const ICON_CUT: &[u8] = include_bytes!("../assets/icons/cut.svg");
pub const ICON_DELETE: &[u8] = include_bytes!("../assets/icons/delete.svg");
pub const ICON_DRIVE: &[u8] = include_bytes!("../assets/icons/drive.svg");
pub const ICON_EXTERNAL_LINK: &[u8] = include_bytes!("../assets/icons/external-link.svg");
pub const ICON_FOLDER: &[u8] = include_bytes!("../assets/icons/folder.svg");
pub const ICON_FOLDER_NEW: &[u8] = include_bytes!("../assets/icons/folder_new.svg");
pub const ICON_HEADPHONES: &[u8] = include_bytes!("../assets/icons/headphones.svg");
pub const ICON_HOME: &[u8] = include_bytes!("../assets/icons/home.svg");
pub const ICON_INFO: &[u8] = include_bytes!("../assets/icons/info.svg");
pub const ICON_LANGUAGES: &[u8] = include_bytes!("../assets/icons/languages.svg");
pub const ICON_MAXIMIZE: &[u8] = include_bytes!("../assets/icons/maximize.svg");
pub const ICON_MINIMIZE: &[u8] = include_bytes!("../assets/icons/minimize.svg");
pub const ICON_MINIMIZE_2: &[u8] = include_bytes!("../assets/icons/minimize_2.svg");
pub const ICON_NAV_BACK: &[u8] = include_bytes!("../assets/icons/nav_back.svg");
pub const ICON_NAV_FORWARD: &[u8] = include_bytes!("../assets/icons/nav_forward.svg");
pub const ICON_NAV_UP: &[u8] = include_bytes!("../assets/icons/nav_up.svg");
pub const ICON_PASTE: &[u8] = include_bytes!("../assets/icons/paste.svg");
pub const ICON_PAUSE: &[u8] = include_bytes!("../assets/icons/pause.svg");
pub const ICON_PLAY: &[u8] = include_bytes!("../assets/icons/play.svg");
pub const ICON_PROPERTIES: &[u8] = include_bytes!("../assets/icons/properties.svg");
pub const ICON_REFRESH: &[u8] = include_bytes!("../assets/icons/refresh.svg");
pub const ICON_RENAME: &[u8] = include_bytes!("../assets/icons/rename.svg");
pub const ICON_SEARCH: &[u8] = include_bytes!("../assets/icons/search.svg");
pub const ICON_VIEW_GRID: &[u8] = include_bytes!("../assets/icons/view_grid.svg");
pub const ICON_VIEW_LIST: &[u8] = include_bytes!("../assets/icons/view_list.svg");
pub const ICON_VOL_HIGH: &[u8] = include_bytes!("../assets/icons/vol_high.svg");
pub const ICON_VOL_MUTE: &[u8] = include_bytes!("../assets/icons/vol_mute.svg");

/// Get embedded SVG icon by name
pub fn get_icon(name: &str) -> Option<&'static [u8]> {
    match name {
        "copy" => Some(ICON_COPY),
        "cut" => Some(ICON_CUT),
        "delete" => Some(ICON_DELETE),
        "drive" => Some(ICON_DRIVE),
        "external-link" => Some(ICON_EXTERNAL_LINK),
        "folder" => Some(ICON_FOLDER),
        "folder_new" => Some(ICON_FOLDER_NEW),
        "headphones" => Some(ICON_HEADPHONES),
        "home" => Some(ICON_HOME),
        "info" => Some(ICON_INFO),
        "languages" => Some(ICON_LANGUAGES),
        "maximize" => Some(ICON_MAXIMIZE),
        "minimize" => Some(ICON_MINIMIZE),
        "minimize_2" => Some(ICON_MINIMIZE_2),
        "nav_back" => Some(ICON_NAV_BACK),
        "nav_forward" => Some(ICON_NAV_FORWARD),
        "nav_up" => Some(ICON_NAV_UP),
        "paste" => Some(ICON_PASTE),
        "pause" => Some(ICON_PAUSE),
        "play" => Some(ICON_PLAY),
        "properties" => Some(ICON_PROPERTIES),
        "refresh" => Some(ICON_REFRESH),
        "rename" => Some(ICON_RENAME),
        "search" => Some(ICON_SEARCH),
        "view_grid" => Some(ICON_VIEW_GRID),
        "view_list" => Some(ICON_VIEW_LIST),
        "vol_high" => Some(ICON_VOL_HIGH),
        "vol_mute" => Some(ICON_VOL_MUTE),
        _ => None,
    }
}
