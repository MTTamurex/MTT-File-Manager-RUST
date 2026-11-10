use super::description;
use crate::app::shortcuts::ShortcutBindings;
use eframe::egui;
use rust_i18n::t;

pub(super) const DESCRIPTION_WIDTH: f32 = 320.0;
const LIST_WIDTH: f32 = 640.0;
const COLUMN_GAP: f32 = 24.0;
const VIEWPORT_CLEARANCE: f32 = 48.0;

pub(super) struct Columns {
    pub list: egui::Rect,
    pub description: egui::Rect,
}

pub(super) fn allocate(ui: &mut egui::Ui, shortcuts: &ShortcutBindings) -> Columns {
    let available = (ui.ctx().viewport_rect().size() - egui::Vec2::splat(VIEWPORT_CLEARANCE))
        .max(egui::Vec2::splat(1.0));
    let width = available.x.min(LIST_WIDTH + COLUMN_GAP + DESCRIPTION_WIDTH);
    // Keep the fixed description width unless the viewport physically cannot accommodate it.
    let gap = COLUMN_GAP.min(width * 0.1);
    let description_width = DESCRIPTION_WIDTH.min((width - gap - 160.0).max(width * 0.35));
    let list_width = width - gap - description_width;
    let text_height = |text: String, style: egui::TextStyle| {
        ui.painter()
            .layout(
                text,
                style.resolve(ui.style()),
                ui.visuals().text_color(),
                list_width,
            )
            .size()
            .y
    };
    let chrome_height = text_height(
        t!("command_palette.title").to_string(),
        egui::TextStyle::Heading,
    ) + ui.text_style_height(&egui::TextStyle::Body) * 2.0
        + 8.0
        + text_height(
            t!("command_palette.hint").to_string(),
            egui::TextStyle::Body,
        )
        + ui.spacing().item_spacing.y * 7.0
        + 12.0;
    let height = (420.0 + chrome_height)
        .max(description::required_height(ui, description_width, shortcuts) + 2.0)
        .min(available.y);
    let (bounds, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let list = egui::Rect::from_min_size(bounds.min, egui::vec2(list_width, height));
    let description = egui::Rect::from_min_size(
        egui::pos2(bounds.right() - description_width, bounds.top()),
        egui::vec2(description_width, height),
    );
    let separator = egui::Rect::from_min_max(
        egui::pos2(list.right(), bounds.top()),
        egui::pos2(description.left(), bounds.bottom()),
    );
    ui.put(separator, egui::Separator::default().vertical());
    Columns { list, description }
}

pub(super) fn column(ui: &mut egui::Ui, rect: egui::Rect, id: &str) -> egui::Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    child
}
