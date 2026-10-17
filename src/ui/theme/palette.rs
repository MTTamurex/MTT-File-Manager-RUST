use crate::app::navigation_state::ThemeMode;
use eframe::egui::{Color32, Context, Stroke, Visuals};

pub fn background(ctx: &Context, legacy: Color32) -> Color32 {
    super::current_palette(ctx).map_or(legacy, |p| p.background)
}

pub fn surface(ctx: &Context, legacy: Color32) -> Color32 {
    super::current_palette(ctx).map_or(legacy, |p| p.surface)
}

pub fn raised(ctx: &Context, legacy: Color32) -> Color32 {
    super::current_palette(ctx).map_or(legacy, |p| p.raised)
}

pub fn hover(ctx: &Context, legacy: Color32) -> Color32 {
    super::current_palette(ctx).map_or(legacy, |p| p.hover)
}

pub fn border(ctx: &Context, legacy: Color32) -> Color32 {
    super::current_palette(ctx).map_or(legacy, |p| p.border)
}

/// Semantic colors shared by the file manager and standalone viewer chrome.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub background: Color32,
    pub surface: Color32,
    pub input: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub active: Color32,
    pub border: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub accent: Color32,
    pub selection: Color32,
    pub selection_text: Color32,
}

pub fn for_mode(mode: ThemeMode) -> Option<Palette> {
    match mode {
        ThemeMode::DarkDeep => Some(Palette {
            background: Color32::from_gray(24),
            surface: Color32::from_gray(30),
            input: Color32::from_gray(20),
            raised: Color32::from_gray(37),
            hover: Color32::from_gray(44),
            active: Color32::from_gray(54),
            border: Color32::from_gray(64),
            text: Color32::from_gray(230),
            secondary: Color32::from_gray(170),
            accent: Color32::from_rgb(96, 173, 255),
            selection: Color32::from_rgb(35, 62, 94),
            selection_text: Color32::from_rgb(225, 237, 255),
        }),
        ThemeMode::GitHubDark => Some(Palette {
            background: Color32::from_rgb(13, 17, 23),
            surface: Color32::from_rgb(22, 27, 34),
            input: Color32::from_rgb(13, 17, 23),
            raised: Color32::from_rgb(33, 38, 45),
            hover: Color32::from_rgb(41, 46, 54),
            active: Color32::from_rgb(48, 54, 61),
            border: Color32::from_rgb(48, 54, 61),
            text: Color32::from_rgb(230, 237, 243),
            secondary: Color32::from_rgb(157, 167, 179),
            accent: Color32::from_rgb(88, 166, 255),
            selection: Color32::from_rgb(31, 58, 96),
            selection_text: Color32::from_rgb(230, 237, 243),
        }),
        _ => None,
    }
}

impl Palette {
    pub fn apply(self, visuals: &mut Visuals) {
        visuals.window_fill = self.surface;
        visuals.panel_fill = self.background;
        visuals.extreme_bg_color = self.input;
        visuals.faint_bg_color = self.surface;
        visuals.code_bg_color = self.surface;
        visuals.window_stroke = Stroke::new(1.0, self.border);
        visuals.override_text_color = Some(self.text);
        visuals.weak_text_color = Some(self.secondary);
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.selection;
        visuals.selection.stroke = Stroke::new(1.0, self.selection_text);

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = self.surface;
        widgets.noninteractive.weak_bg_fill = self.surface;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        for (widget, fill) in [
            (&mut widgets.inactive, self.raised),
            (&mut widgets.hovered, self.hover),
            (&mut widgets.active, self.active),
            (&mut widgets.open, self.raised),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, self.border);
            widget.fg_stroke = Stroke::new(1.0, self.text);
        }
    }
}
