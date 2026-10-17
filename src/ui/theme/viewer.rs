use eframe::egui::{Color32, CornerRadius, Stroke, Visuals};

/// Preserve the original light/dark viewer styling as the base for all palettes.
pub(super) fn viewer_visuals(dark_mode: bool) -> Visuals {
    let mut visuals = if dark_mode {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    if dark_mode {
        visuals.window_fill = Color32::from_gray(32);
        visuals.panel_fill = Color32::from_gray(28);
        visuals.extreme_bg_color = Color32::from_gray(24);
        visuals.window_stroke = Stroke::new(1.0, Color32::from_gray(60));
        visuals.widgets.noninteractive.bg_fill = Color32::from_gray(37);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(60));
        visuals.widgets.inactive.bg_fill = Color32::from_gray(45);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_gray(45);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(64));
        visuals.widgets.hovered.bg_fill = Color32::from_gray(58);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_gray(58);
        visuals.widgets.active.bg_fill = Color32::from_gray(70);
        visuals.widgets.active.weak_bg_fill = Color32::from_gray(70);
    } else {
        visuals.window_fill = Color32::from_rgb(246, 246, 246);
        visuals.panel_fill = Color32::from_rgb(243, 243, 243);
        visuals.extreme_bg_color = Color32::WHITE;
        visuals.window_stroke = Stroke::new(1.0, Color32::from_gray(220));
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(225));
        visuals.widgets.inactive.bg_fill = Color32::WHITE;
        visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(214));
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(229, 243, 255);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(229, 243, 255);
        visuals.widgets.active.bg_fill = Color32::from_rgb(204, 228, 247);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(204, 228, 247);
    }
    let radius = CornerRadius::same(6);
    visuals.window_corner_radius = radius;
    visuals.menu_corner_radius = radius;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals
}
