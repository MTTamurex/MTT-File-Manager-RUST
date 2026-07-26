use eframe::egui;
use std::sync::LazyLock;

const FILLED_TAG_ICON: &str = "\u{f022}"; // Remix Icon: price-tag-3-fill

static ICON_FONT_FAMILY: LazyLock<egui::FontFamily> =
    LazyLock::new(|| egui::FontFamily::Name("icons".into()));

pub(crate) fn paint_filled(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    color: egui::Color32,
) {
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        FILLED_TAG_ICON,
        egui::FontId::new(size, ICON_FONT_FAMILY.clone()),
        color,
    );
}

pub(crate) fn paint_filled_badge(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    color: egui::Color32,
    outline: egui::Color32,
) {
    paint_filled(painter, center, size + 1.5, outline);
    paint_filled(painter, center, size, color);
}
