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
pub const COLOR_ACCENT: Color32 = Color32::from_rgb(0, 120, 215);

pub fn color_hover() -> Color32 {
    Color32::from_rgba_unmultiplied(200, 220, 240, 50)
}

// === COLORS (Dark Mode) ===
pub const COLOR_DARK_BG: Color32 = Color32::from_rgb(45, 45, 45);

pub fn color_dark_hover() -> Color32 {
    Color32::from_white_alpha(30)
}

// === TIMING ===
pub const DEBOUNCE_MS: u64 = 50;
pub const DRIVE_REFRESH_MS: u64 = 350;
pub const AUTO_RELOAD_MS: u64 = 500;

// === CACHE SIZES ===
pub const TEXTURE_CACHE_SIZE: usize = 200;
pub const ICON_CACHE_SIZE: usize = 100;
pub const METADATA_CACHE_SIZE: usize = 512;
