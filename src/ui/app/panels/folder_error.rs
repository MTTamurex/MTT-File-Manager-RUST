use crate::app::state::{FolderLoadErrorKind, ImageViewerApp};
use crate::infrastructure::windows::folder_access::AccessError;
use eframe::egui;

pub(super) fn render(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let Some(error) = app.folder_load_error.clone() else {
        return;
    };
    let message = match error.kind {
        FolderLoadErrorKind::AccessDenied => rust_i18n::t!("panels.folder_access_denied"),
        FolderLoadErrorKind::NotFound => rust_i18n::t!("panels.folder_not_found"),
        FolderLoadErrorKind::Other => rust_i18n::t!("panels.folder_load_failed"),
    };
    let pending = app.folder_access_pending();
    let status = app.folder_access_status();
    let mut proceed = false;
    ui.centered_and_justified(|ui| {
        ui.vertical_centered(|ui| {
            ui.label(message.to_string());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(error.path.display().to_string())
                    .size(11.0)
                    .color(egui::Color32::from_gray(130)),
            );
            if error.kind == FolderLoadErrorKind::AccessDenied {
                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new(rust_i18n::t!("panels.folder_access_warning_title"))
                        .strong()
                        .color(ui.visuals().warn_fg_color),
                );
                ui.add_sized(
                    [ui.available_width().min(560.0), 0.0],
                    egui::Label::new(rust_i18n::t!("panels.folder_access_warning")).wrap(),
                );
                ui.add_space(8.0);
                ui.label(rust_i18n::t!("panels.folder_access_explanation"));
                proceed = ui
                    .add_enabled(
                        !pending,
                        egui::Button::new(format!(
                            "🛡 {}",
                            rust_i18n::t!("panels.folder_access_continue")
                        )),
                    )
                    .clicked();
                if pending {
                    ui.spinner();
                    ui.label(rust_i18n::t!("panels.folder_access_pending"));
                } else if let Some(status) = status {
                    ui.label(match status {
                        AccessError::Cancelled => rust_i18n::t!("panels.folder_access_cancelled"),
                        AccessError::Failed(code) => {
                            rust_i18n::t!("panels.folder_access_failed", code = code)
                        }
                    });
                }
            }
        });
    });
    if proceed {
        app.start_folder_access();
    }
}
