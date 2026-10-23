use crate::ui::components::settings_ui;
use crate::ui::fonts::selection;
use eframe::egui;
use rust_i18n::t;

mod selector;
mod size_control;

pub(super) fn render(ui: &mut egui::Ui, db: &crate::infrastructure::app_state_db::AppStateDb) {
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
    let snapshot = selection::snapshot(ui.ctx()).unwrap_or(snapshot);
    if snapshot.loading {
        ui.spinner();
    }
    ui.add_space(12.0);
    ui.label(t!("settings.ui_font_hint"));
    ui.add_space(12.0);
    if let Some(scale) = size_control::render(
        ui,
        &snapshot.selected,
        snapshot.scale,
        !output.open && !snapshot.loading,
    )
    .0
    {
        selection::set_scale(ui.ctx(), db, scale);
    }
    ui.add_space(8.0);
    ui.label(t!("settings.ui_font_scale_hint"));
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
