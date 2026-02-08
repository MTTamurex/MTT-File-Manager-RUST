use crate::app::ImageViewerApp;
use crate::ui::theme;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SecAction {
    None,
    Cut,
    Copy,
    Paste,
    Rename,
    CreateFolder,
    Delete,
}

pub(super) fn render_action_buttons(ui: &mut egui::Ui, app: &mut ImageViewerApp) -> SecAction {
    let icon_size = egui::vec2(28.0, 28.0);
    let is_drive_selected = app
        .selected_file
        .as_ref()
        .is_some_and(|f| f.drive_info.is_some());
    let has_selection =
        (app.selected_file.is_some() || !app.multi_selection.is_empty()) && !is_drive_selected;
    let can_rename = app.multi_selection.len() <= 1
        && (app.multi_selection.len() == 1 || app.selected_file.is_some());
    let can_paste = app.clipboard.has_content() && !is_drive_selected;
    let can_create_folder = !app.is_computer_view && !app.is_recycle_bin_view;

    let icon_color = if ui.visuals().dark_mode {
        [220, 220, 220, 255]
    } else {
        [60, 60, 60, 255]
    };
    let disabled_color = [128, 128, 128, 180];
    let mut action = SecAction::None;

    {
        let svg_manager = &mut app.svg_icon_manager;

        let mut render_btn = |icon_name: &str, enabled: bool, tooltip: &str| -> bool {
            let color = if enabled { icon_color } else { disabled_color };
            let sense = if enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            };
            let (rect, response) = ui.allocate_exact_size(icon_size, sense);

            if enabled && response.hovered() {
                let bg_color = if ui.visuals().dark_mode {
                    theme::color_dark_hover()
                } else {
                    theme::color_hover()
                };
                ui.painter().rect_filled(rect, 6.0, bg_color);
            }

            if let Some(texture) = svg_manager.get_icon(ui.ctx(), icon_name, 32, color) {
                let display_size = if icon_name == "folder_new" {
                    18.0
                } else {
                    16.0
                };
                let icon_rect = egui::Rect::from_center_size(
                    rect.center(),
                    egui::vec2(display_size, display_size),
                );
                ui.painter().image(
                    texture.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            } else {
                let fallback = icon_name.chars().next().unwrap_or('?').to_string();
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    fallback,
                    egui::FontId::proportional(12.0),
                    egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                );
            }

            let response = if enabled {
                response.on_hover_cursor(egui::CursorIcon::PointingHand)
            } else {
                response
            };

            if enabled {
                response.on_hover_text(tooltip).clicked()
            } else {
                response.on_hover_text(format!("{} (Desabilitado)", tooltip));
                false
            }
        };

        if render_btn("cut", has_selection, "Recortar (Ctrl+X)") {
            action = SecAction::Cut;
        }
        if render_btn("copy", has_selection, "Copiar (Ctrl+C)") {
            action = SecAction::Copy;
        }
        if render_btn("paste", can_paste, "Colar (Ctrl+V)") {
            action = SecAction::Paste;
        }
        if render_btn("rename", can_rename, "Renomear (F2)") {
            action = SecAction::Rename;
        }
        if render_btn(
            "folder_new",
            can_create_folder,
            "Criar Nova Pasta (Ctrl+Shift+N)",
        ) {
            action = SecAction::CreateFolder;
        }
        if render_btn("delete", has_selection, "Excluir (Del)") {
            action = SecAction::Delete;
        }
    }

    action
}

pub(super) fn execute_action(action: SecAction, app: &mut ImageViewerApp) {
    match action {
        SecAction::Cut => app.command_cut(Option::from(app.selected_item)),
        SecAction::Copy => app.command_copy(Option::from(app.selected_item)),
        SecAction::Paste => app.command_paste(None),
        SecAction::Rename => {
            if let Some(idx) = app.selected_item {
                if let Some(item) = app.items.get(idx) {
                    app.renaming_state = Some((idx, item.name.clone()));
                    app.focus_rename = true;
                }
            }
        }
        SecAction::CreateFolder => app.create_new_folder(),
        SecAction::Delete => {
            let mut targets = Vec::new();
            if app.multi_selection.is_empty() {
                if let Some(idx) = app.selected_item {
                    if let Some(item) = app.items.get(idx) {
                        targets.push(item.path.clone());
                    }
                }
            } else {
                targets.extend(app.multi_selection.iter().cloned());
            }

            if !targets.is_empty() {
                app.delete_with_shell_for_paths(&targets);
            }
        }
        SecAction::None => {}
    }
}
