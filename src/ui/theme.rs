use eframe::egui::Color32;

// === SPACING ===
pub const PADDING_XS: f32 = 2.0;
pub const PADDING_SM: f32 = 4.0;
pub const PADDING_MD: f32 = 8.0;
pub const PADDING_LG: f32 = 12.0;
pub const PADDING_XL: f32 = 16.0;

// === SIZES ===
pub const ICON_SIZE_SM: f32 = 16.0;
pub const ICON_SIZE_MD: f32 = 22.0;
pub const ICON_SIZE_LG: f32 = 24.0;
pub const THUMBNAIL_MIN: f32 = 64.0;
pub const THUMBNAIL_MAX: f32 = 512.0;
pub const THUMBNAIL_DEFAULT: f32 = 128.0;

// === COLORS (Light Mode) ===
pub const COLOR_SELECTION: Color32 = Color32::from_rgb(200, 220, 240);
pub const COLOR_SELECTION_TEXT: Color32 = Color32::from_rgb(0, 50, 100);

pub fn color_selection_hover() -> Color32 {
    // Feedback: "Still white" (with transparency).
    // Definitive Solution: SOLID color (No transparency) to ensure the tone.
    // Windows 10/11 Standard List View Hover: R=229, G=243, B=255.
    // It's a very light blue, but being solid, it doesn't blend with the background.
    Color32::from_rgb(229, 243, 255)
}
pub const COLOR_ACCENT: Color32 = Color32::from_rgb(0, 120, 215);

pub fn color_hover() -> Color32 {
    Color32::from_black_alpha(15)
}

// === COLORS (Dark Mode) ===
pub const COLOR_DARK_BG: Color32 = Color32::from_rgb(45, 45, 45);

pub fn color_dark_hover() -> Color32 {
    Color32::from_white_alpha(25)
}

// === TIMING ===
pub const DEBOUNCE_MS: u64 = 50;
pub const DRIVE_REFRESH_MS: u64 = 350;
pub const AUTO_RELOAD_MS: u64 = 200;

// === CACHE SIZES ===
pub const TEXTURE_CACHE_SIZE: usize = 200;
pub const ICON_CACHE_SIZE: usize = 100;

pub const METADATA_CACHE_SIZE: usize = 512;
// === ICONS (Remix Icon Mappings) ===
pub const ICON_ARROW_LEFT: &str = "\u{EA64}"; // Left Arrow
pub const ICON_ARROW_RIGHT: &str = "\u{EA6E}"; // Right Arrow
pub const ICON_ARROW_UP: &str = "\u{EA78}"; // Up Arrow
pub const ICON_REFRESH: &str = "\u{F064}"; // Refresh
pub const ICON_HOME: &str = "\u{EE1B}"; // Home/PC
pub const ICON_GRID: &str = "\u{ED9E}"; // Grid
pub const ICON_LIST: &str = "\u{EF3E}"; // List
pub const ICON_SEARCH: &str = "\u{F0D1}"; // Magnifier
pub const ICON_FOLDER_ADD: &str = "\u{ED5A}"; // New Folder
pub const ICON_DETAILS: &str = "\u{ECEA}"; // Details (file-info-line)
pub const ICON_FOLDER: &str = "\u{ED9F}"; // Folder (folder-line)
pub const ICON_FILE: &str = "\u{ECD3}"; // File (file-line)
