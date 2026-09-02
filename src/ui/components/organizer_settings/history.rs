use crate::app::ImageViewerApp;
use crate::domain::organizer_operation::{OrganizerOperationStatus, OrganizerOperationType};
use crate::infrastructure::app_state_db::{
    MAX_ORGANIZER_HISTORY_RETENTION_DAYS, MIN_ORGANIZER_HISTORY_RETENTION_DAYS,
};
use crate::ui::components::settings_ui;
use crate::ui::theme;
use eframe::egui::{self, RichText};
use rust_i18n::t;

const RECOMMENDED_RETENTION_DAYS: [i64; 4] = [7, 30, 90, 180];

pub(super) fn render_history(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) {
    ui.add_space(18.0);
    settings_ui::card_frame(dark_mode).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            settings_ui::sub_header(ui, &t!("organizer.history_title"));
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    app.organizer_state
                        .history_state
                        .operations
                        .len()
                        .to_string(),
                )
                .small()
                .color(theme::secondary_text_color(dark_mode)),
            );
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(t!("organizer.history_description"))
                .size(13.0)
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.add_space(10.0);
        render_retention(ui, app, dark_mode);
        ui.add_space(12.0);

        if let Some(error) = app.organizer_state.history_state.load_error.clone() {
            ui.label(
                RichText::new(t!("organizer.history_load_failed", reason = error))
                    .color(theme::secondary_text_color(dark_mode)),
            );
            if ui.button(t!("organizer.retry_history")).clicked() {
                if let Err(error) = app.organizer_state.reload_history(&app.app_state_db) {
                    app.notifications.warning(error.to_string());
                }
            }
            return;
        }

        if app.organizer_state.history_state.operations.is_empty() {
            ui.label(
                RichText::new(t!("organizer.no_history"))
                    .color(theme::secondary_text_color(dark_mode)),
            );
            return;
        }

        let operations = app.organizer_state.history_state.operations.clone();
        for operation in operations {
            render_operation_row(ui, app, dark_mode, operation);
            ui.add_space(8.0);
        }
    });
}

fn render_retention(ui: &mut egui::Ui, app: &mut ImageViewerApp, dark_mode: bool) {
    let current = app.organizer_state.history_state.retention_days;
    let mut selected = current;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(t!("organizer.history_retention"))
                .color(theme::secondary_text_color(dark_mode)),
        );
        egui::ComboBox::from_id_salt("organizer_history_retention")
            .selected_text(format_retention_label(current))
            .show_ui(ui, |ui| {
                for days in RECOMMENDED_RETENTION_DAYS {
                    ui.selectable_value(&mut selected, days, format_retention_label(days));
                }
            });
        let mut custom = app.organizer_state.history_state.retention_input.clone();
        let response = ui.add(
            egui::TextEdit::singleline(&mut custom)
                .desired_width(64.0)
                .hint_text(t!("organizer.history_retention_custom")),
        );
        app.organizer_state.history_state.retention_input = custom.clone();
        if response.changed() {
            app.organizer_state.history_state.retention_input_dirty = true;
        }
        if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            match custom.trim().parse::<i64>() {
                Ok(days)
                    if (MIN_ORGANIZER_HISTORY_RETENTION_DAYS
                        ..=MAX_ORGANIZER_HISTORY_RETENTION_DAYS)
                        .contains(&days) =>
                {
                    selected = days;
                }
                _ => app.notifications.warning(
                    t!(
                        "organizer.history_retention_invalid",
                        min = MIN_ORGANIZER_HISTORY_RETENTION_DAYS,
                        max = MAX_ORGANIZER_HISTORY_RETENTION_DAYS,
                    )
                    .to_string(),
                ),
            }
        }
    });
    if selected != current {
        match app
            .app_state_db
            .set_organizer_history_retention_days(selected)
        {
            Ok(()) => {
                app.organizer_state.history_state.retention_days = selected;
                app.organizer_state.history_state.retention_input = selected.to_string();
                app.organizer_state.history_state.retention_input_dirty = false;
            }
            Err(error) => app.notifications.warning(error.to_string()),
        }
    }
}

fn render_operation_row(
    ui: &mut egui::Ui,
    app: &mut ImageViewerApp,
    dark_mode: bool,
    operation: crate::infrastructure::app_state_db::OrganizerOperationRecord,
) {
    let operation_id = operation.operation_id;
    let child_pending = app
        .organizer_state
        .history_state
        .operations
        .iter()
        .any(|candidate| {
            candidate.original_operation_id == Some(operation_id)
                && candidate.status == OrganizerOperationStatus::Started
        });
    let pending = app
        .organizer_state
        .is_history_operation_pending(operation_id)
        || app
            .organizer_state
            .active_operation_ids
            .contains(&operation_id)
        || child_pending;
    let can_retry = !pending
        && operation.operation_type != OrganizerOperationType::Undo
        && operation.rule_id.is_some()
        && operation.source_snapshot_before.is_some()
        && matches!(
            operation.status,
            OrganizerOperationStatus::Failed
                | OrganizerOperationStatus::Skipped
                | OrganizerOperationStatus::Cancelled
        );
    let already_undone = app
        .organizer_state
        .history_state
        .operations
        .iter()
        .any(|candidate| {
            candidate.original_operation_id == Some(operation_id)
                && candidate.operation_type == OrganizerOperationType::Undo
                && matches!(
                    candidate.status,
                    OrganizerOperationStatus::Started | OrganizerOperationStatus::Completed
                )
        });
    let can_undo = !pending
        && !already_undone
        && operation.undone_at.is_none()
        && operation.operation_type != OrganizerOperationType::Undo
        && operation.destination_snapshot_after.is_some()
        && operation.status == OrganizerOperationStatus::Completed;
    let mut retry = false;
    let mut undo = false;

    settings_ui::row_frame(dark_mode).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(t!("organizer.operation_id", id = operation_id.get()))
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );
            ui.label(
                RichText::new(operation_type_label(operation.operation_type))
                    .small()
                    .color(theme::secondary_text_color(dark_mode)),
            );
            ui.label(
                RichText::new(operation_status_label(operation.status))
                    .small()
                    .color(theme::secondary_text_color(dark_mode)),
            );
        });
        path_row(
            ui,
            dark_mode,
            &t!("organizer.operation_source"),
            operation
                .effective_source_path
                .as_ref()
                .unwrap_or(&operation.source_path),
        );
        path_row(
            ui,
            dark_mode,
            &t!("organizer.operation_destination"),
            operation
                .effective_destination_path
                .as_ref()
                .unwrap_or(&operation.destination_path),
        );
        if let Some(error) = operation.error.as_deref() {
            ui.label(
                RichText::new(t!("organizer.operation_error", reason = error))
                    .small()
                    .color(theme::secondary_text_color(dark_mode)),
            );
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            if can_retry
                && ui
                    .button(t!("organizer.retry_operation"))
                    .on_hover_text(t!("organizer.retry_operation_hint"))
                    .clicked()
            {
                retry = true;
            }
            if can_undo && ui.button(t!("organizer.undo_operation")).clicked() {
                undo = true;
            }
            if pending {
                ui.spinner();
                ui.label(t!("organizer.operation_action_pending"));
            }
        });
    });

    if retry {
        if let Err(error) = app.organizer_state.retry_operation(operation_id) {
            app.notifications.warning(error.to_string());
        }
    } else if undo {
        if let Err(error) = app.organizer_state.undo_operation(operation_id) {
            app.notifications.warning(error.to_string());
        }
    }
}

fn path_row(ui: &mut egui::Ui, dark_mode: bool, label: &str, path: &std::path::Path) {
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(label)
                .small()
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.label(
            RichText::new(path.display().to_string())
                .small()
                .monospace()
                .color(theme::text_color(dark_mode)),
        );
    });
}

fn format_retention_label(days: i64) -> String {
    if days == 1 {
        t!("organizer.history_retention_day").to_string()
    } else {
        t!("organizer.history_retention_days", days = days).to_string()
    }
}

fn operation_type_label(operation_type: OrganizerOperationType) -> String {
    match operation_type {
        OrganizerOperationType::Move => t!("organizer.operation_type_move").to_string(),
        OrganizerOperationType::Retry => t!("organizer.operation_type_retry").to_string(),
        OrganizerOperationType::Undo => t!("organizer.operation_type_undo").to_string(),
    }
}

fn operation_status_label(status: OrganizerOperationStatus) -> String {
    match status {
        OrganizerOperationStatus::Started => t!("organizer.operation_status_started").to_string(),
        OrganizerOperationStatus::Completed => {
            t!("organizer.operation_status_completed").to_string()
        }
        OrganizerOperationStatus::Skipped => t!("organizer.operation_status_skipped").to_string(),
        OrganizerOperationStatus::Cancelled => {
            t!("organizer.operation_status_cancelled").to_string()
        }
        OrganizerOperationStatus::Failed => t!("organizer.operation_status_failed").to_string(),
    }
}
