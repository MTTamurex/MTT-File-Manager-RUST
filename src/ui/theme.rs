use eframe::egui::{self, Color32};

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
pub const THUMBNAIL_MIN: f32 = 96.0;
pub const THUMBNAIL_MAX: f32 = 512.0;
pub const THUMBNAIL_DEFAULT: f32 = 128.0;
pub const MIN_GRID_THUMBNAIL_BUCKET: u32 = 512;

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
pub const COLOR_DARK_SELECTION: Color32 = Color32::from_rgb(40, 60, 90);
pub const COLOR_DARK_SELECTION_TEXT: Color32 = Color32::from_rgb(200, 220, 240);

pub fn color_dark_selection_hover() -> Color32 {
    Color32::from_rgb(50, 60, 70)
}

pub fn color_dark_hover() -> Color32 {
    Color32::from_white_alpha(25)
}

// === Dark-mode-aware color access ===
pub fn text_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_gray(220)
    } else {
        Color32::BLACK
    }
}

pub fn secondary_text_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_gray(160)
    } else {
        Color32::from_gray(100)
    }
}

pub fn input_bg_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_gray(55)
    } else {
        Color32::WHITE
    }
}

pub fn selection_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        COLOR_DARK_SELECTION
    } else {
        COLOR_SELECTION
    }
}

pub fn selection_text_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        COLOR_DARK_SELECTION_TEXT
    } else {
        COLOR_SELECTION_TEXT
    }
}

pub fn header_active_bg(dark_mode: bool) -> Color32 {
    if dark_mode {
        Color32::from_gray(60)
    } else {
        Color32::from_gray(230)
    }
}

pub fn selection_hover_color(dark_mode: bool) -> Color32 {
    if dark_mode {
        color_dark_selection_hover()
    } else {
        color_selection_hover()
    }
}

// === TIMING ===
pub const DEBOUNCE_MS: u64 = 50;
pub const DRIVE_REFRESH_MS: u64 = 350;
pub const AUTO_RELOAD_MS: u64 = 200;

// === CACHE SIZES ===
pub const TEXTURE_CACHE_SIZE: usize = 200;
pub const ICON_CACHE_SIZE: usize = 100;

pub const METADATA_CACHE_SIZE: usize = 512;

// === SCROLL STYLE ===

/// Apply a unified floating scrollbar style: thin by default, expands on hover,
/// neutral gray handle (dark and light mode). Call after every `ctx.set_visuals()`.
pub fn apply_scroll_style(ctx: &egui::Context) {
    ctx.style_mut(|style| {
        let s = &mut style.spacing.scroll;
        s.floating = true;
        s.bar_width = 8.0; // Max width when hovered
        s.floating_width = 3.0; // Thin resting width
        s.floating_allocated_width = 0.0;
        s.handle_min_length = 20.0;
        s.bar_inner_margin = 2.0;
        s.bar_outer_margin = 0.0;
        // Use foreground (fg_stroke) color — always contrasts with background
        s.foreground_color = true;
        // Resting: nearly invisible
        s.dormant_background_opacity = 0.0;
        s.dormant_handle_opacity = 0.0;
        // Active (pointer in scroll area): subtle hint
        s.active_background_opacity = 0.0;
        s.active_handle_opacity = 0.4;
        // Interacting (hovering/dragging the bar itself): clearly visible
        s.interact_background_opacity = 0.04;
        s.interact_handle_opacity = 0.7;
    });
}

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
pub const ICON_VIEW_COLUMNS: &str = "view_columns";
pub const ICON_VIEW_DETAILS: &str = "view_details";
pub const ICON_FOLDER: &str = "\u{ED9F}"; // Folder (folder-line)
pub const ICON_FILE: &str = "\u{ECD3}"; // File (file-line)
