use crate::ui::components::settings_ui;
use crate::ui::fonts::selection;
use eframe::egui;
use rust_i18n::t;

mod selector;

pub(super) fn render(ui: &mut egui::Ui) {
    settings_ui::subsection_header(ui, &t!("settings.ui_font"), "");
    let Some(snapshot) = selection::snapshot(ui.ctx()) else {
        ui.spinner();
        ui.label(t!("settings.ui_font_loading"));
        return;
    };
    let output = ui
        .add_enabled_ui(!snapshot.committing, |ui| {
            selector::render(ui, &snapshot.catalog.families, &snapshot.selected)
        })
        .inner;
    if output.cancelled {
        selection::cancel_preview(ui.ctx());
    }
    if let Some(selected) = output.confirmed {
        selection::request(ui.ctx(), selected);
    } else if output.open {
        selection::preview_frame(ui.ctx());
        if let Some(selected) = output.preview {
            selection::preview(ui.ctx(), selected);
        }
    }
    if snapshot.loading {
        ui.spinner();
    }
    ui.add_space(12.0);
    ui.label(t!("settings.ui_font_hint"));
    if snapshot.unavailable {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            t!("settings.ui_font_unavailable"),
        );
    }
    if snapshot.save_failed {
        ui.colored_label(
            ui.visuals().error_fg_color,
            t!("settings.ui_font_save_failed"),
        );
    }
}
