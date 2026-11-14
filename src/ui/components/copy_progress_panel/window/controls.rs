//! Minimize keeps operations running; Close cancels active jobs before hiding.
use eframe::egui;
use rust_i18n::t;

pub(super) const MINIMIZE_ID: &str = "copy_window_minimize";
pub(super) const CLOSE_ID: &str = "copy_window_close";

fn close(ui: &mut egui::Ui, active: bool) -> egui::Response {
    button(
        ui,
        CLOSE_ID,
        &if active {
            t!("copy_engine.cancel_and_close")
        } else {
            t!("copy_engine.dismiss")
        },
    )
}

fn button(ui: &mut egui::Ui, id: &str, tooltip: &str) -> egui::Response {
    // Exact, identical sizes independent of font glyph metrics and padding.
    let (_, rect) = ui.allocate_space(egui::vec2(26.0, 20.0));
    let response = ui.interact(rect, egui::Id::new(id), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let center = rect.center();
    let stroke = egui::Stroke::new(1.5, visuals.fg_stroke.color);
    if id == MINIMIZE_ID {
        ui.painter().line_segment(
            [center - egui::vec2(4.0, 0.0), center + egui::vec2(4.0, 0.0)],
            stroke,
        );
    } else {
        for direction in [-1.0, 1.0] {
            let offset = egui::vec2(3.5, direction * 3.5);
            ui.painter()
                .line_segment([center - offset, center + offset], stroke);
        }
    }
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
            minimize = button(ui, MINIMIZE_ID, &t!("copy_engine.minimize")).clicked();
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
