//! Familiar window controls; hiding never changes an operation's state.
use eframe::egui;
use rust_i18n::t;

pub(super) const MINIMIZE: &str = "—";
const CLOSE: &str = "×";

pub(in crate::ui::components::copy_progress_panel) fn close(
    ui: &mut egui::Ui,
    active: bool,
) -> egui::Response {
    button(
        ui,
        CLOSE,
        &if active {
            t!("copy_engine.hide_window")
        } else {
            t!("copy_engine.dismiss")
        },
    )
}

fn button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    let response =
        ui.add(egui::Button::new(icon).min_size(egui::vec2(32.0, ui.spacing().interact_size.y)));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
    });
    response.on_hover_text(tooltip)
}

pub(super) fn header(ui: &mut egui::Ui, title: &str, active: bool) -> (bool, bool) {
    let mut minimize = false;
    let mut close_clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            close_clicked = close(ui, active).clicked();
            minimize = button(ui, MINIMIZE, &t!("copy_engine.minimize")).clicked();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(title).text_style(egui::TextStyle::Button),
                    )
                    .truncate()
                    .selectable(false),
                );
            });
        });
    });
    ui.separator();
    (minimize, close_clicked)
}
