use crate::app::ImageViewerApp;
use crate::ui::theme;
use eframe::egui;
use rust_i18n::t;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SecAction {
    None,
    Cut,
    Copy,
    Paste,
    Rename,
    CreateFolder,
    Delete,
    EmptyRecycleBin,
}

pub(super) fn render_action_buttons(ui: &mut egui::Ui, app: &mut ImageViewerApp) -> SecAction {
    let icon_size = egui::vec2(28.0, 28.0);
    let is_recycle_bin_view = app.navigation_state.is_recycle_bin_view;
    let is_drive_selected = app
        .selected_file
        .as_ref()
        .is_some_and(|f| f.drive_info.is_some());
    let has_selection =
        (app.selected_file.is_some() || !app.multi_selection.is_empty()) && !is_drive_selected;
    let in_archive_namespace = app.current_location_is_archive_namespace();
    let selection_cannot_move = app.multi_selection.iter().any(|path| {
        crate::domain::file_entry::is_path_inside_existing_archive_file(path)
            || app.path_is_same_or_ancestor_of_open_panel(path)
    }) || app.selected_file.as_ref().is_some_and(|item| {
        crate::domain::file_entry::is_path_inside_existing_archive_file(&item.path)
            || app.path_is_same_or_ancestor_of_open_panel(&item.path)
    });
    let can_cut = has_selection && !selection_cannot_move;
    let can_copy = has_selection && app.can_copy_from_current_location();
    let can_rename = app.multi_selection.len() <= 1
        && !in_archive_namespace
        && app
            .selected_item
            .is_some_and(|idx| app.can_rename_item(idx));
    let can_paste = app.can_paste_into_current_location() && !is_drive_selected;
    let can_create_folder =
        !crate::domain::special_paths::is_virtual_path(&app.navigation_state.current_path)
            && !in_archive_namespace;
    let can_delete = has_selection && !in_archive_namespace && !selection_cannot_move;
    let can_empty_recycle_bin = is_recycle_bin_view && !app.items.is_empty();

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
                let display_size = if matches!(icon_name, "folder_new" | "broom") {
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
                response.on_hover_text(format!(
                    "{} {}",
                    tooltip,
                    t!("secondary_toolbar.disabled_suffix")
                ));
                false
            }
        };

        if render_btn("cut", can_cut, &t!("secondary_toolbar.cut")) {
            action = SecAction::Cut;
        }
        if render_btn("copy", can_copy, &t!("secondary_toolbar.copy")) {
            action = SecAction::Copy;
        }
        if render_btn("paste", can_paste, &t!("secondary_toolbar.paste")) {
            action = SecAction::Paste;
        }
        if render_btn("rename", can_rename, &t!("secondary_toolbar.rename")) {
            action = SecAction::Rename;
        }
        if render_btn(
            "folder_new",
            can_create_folder,
            &t!("secondary_toolbar.create_folder"),
        ) {
            action = SecAction::CreateFolder;
        }
        if render_btn("delete", can_delete, &t!("secondary_toolbar.delete")) {
            action = SecAction::Delete;
        }
        if is_recycle_bin_view
            && render_btn(
                "broom",
                can_empty_recycle_bin,
                &t!("secondary_toolbar.empty_recycle_bin"),
            )
        {
            action = SecAction::EmptyRecycleBin;
        }
    }

    action
}

pub(super) fn execute_action(action: SecAction, app: &mut ImageViewerApp) {
    match action {
        SecAction::Cut => app.command_cut(app.selected_item),
        SecAction::Copy => app.command_copy(app.selected_item),
        SecAction::Paste => app.command_paste(None),
        SecAction::Rename => {
            if let Some(idx) = app.selected_item {
                app.begin_rename_item(idx);
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
                if app.navigation_state.is_recycle_bin_view {
                    app.delete_permanently(&targets);
                } else {
                    app.delete_with_shell_for_paths(&targets);
                }
            }
        }
        SecAction::EmptyRecycleBin => app.empty_recycle_bin(),
        SecAction::None => {}
    }
}
