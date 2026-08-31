use crate::app::ImageViewerApp;
use crate::infrastructure::organizer::OrganizerRuleStatus;
use eframe::egui;
use rust_i18n::t;

pub(super) fn status_label(status: OrganizerRuleStatus) -> String {
    match status {
        OrganizerRuleStatus::Starting => t!("organizer.status_starting").to_string(),
        OrganizerRuleStatus::Active => t!("organizer.status_active").to_string(),
        OrganizerRuleStatus::Disabled => t!("organizer.status_disabled").to_string(),
        OrganizerRuleStatus::Paused => t!("organizer.status_paused").to_string(),
        OrganizerRuleStatus::SourceUnavailable => {
            t!("organizer.status_source_unavailable").to_string()
        }
        OrganizerRuleStatus::DestinationUnavailable => {
            t!("organizer.status_destination_unavailable").to_string()
        }
        OrganizerRuleStatus::BothUnavailable => t!("organizer.status_both_unavailable").to_string(),
        OrganizerRuleStatus::Recovering => t!("organizer.status_recovering").to_string(),
    }
}

pub(super) fn render_folder_creation_confirmation(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(request) = app.organizer_state.folder_creation_confirmation.clone() else {
        return;
    };
    let Some(rule) = app
        .organizer_state
        .rules
        .iter()
        .find(|rule| rule.id == request.rule_id)
    else {
        app.organizer_state.folder_creation_confirmation = None;
        return;
    };
    let folder = if request.source {
        &rule.source_folder
    } else {
        &rule.destination_folder
    };
    let path = folder.display().to_string();
    let mut confirm = false;
    let mut cancel = false;
    egui::Window::new(t!("organizer.create_folder_title"))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label(t!("organizer.create_folder_message", path = path));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t!("organizer.create")).clicked() {
                    confirm = true;
                }
                if ui.button(t!("organizer.not_now")).clicked() {
                    cancel = true;
                }
            });
        });

    if confirm {
        app.organizer_state.folder_creation_confirmation = None;
        app.organizer_state
            .manager
            .create_missing_folder(request.rule_id, request.source);
    } else if cancel {
        app.organizer_state.folder_creation_confirmation = None;
    }
}
