use super::run_in_context_panel;
use crate::app::ImageViewerApp;
use crate::application::context_menu::ContextMenuState;
use std::path::Path;

fn folder_target(menu: &ContextMenuState) -> Option<&Path> {
    if !menu.is_empty_area {
        return None;
    }
    menu.operation_directory
        .as_deref()
        .or_else(|| menu.target_paths.first().map(|path| path.as_path()))
}

pub(super) fn handle(app: &mut ImageViewerApp, menu: &ContextMenuState, command: &str) -> bool {
    if matches!(command, "open_cmd" | "open_cmd_admin" | "folder_properties") {
        if let Some(path) = folder_target(menu) {
            let path = app.resolve_network_location_path(path);
            if crate::app::operations::file_operation_policy::path_accepts_file_children(&path) {
                if command == "folder_properties" {
                    app.show_properties_for_paths(vec![path]);
                } else if let Err(error) = crate::infrastructure::windows::command_prompt::open(
                    &path,
                    command == "open_cmd_admin",
                    app.native_hwnd,
                ) {
                    app.notifications.error(
                        rust_i18n::t!("context_menu.cmd_error", error = error.to_string())
                            .to_string(),
                    );
                }
            }
        }
        return true;
    }
    if command == "refresh_view"
        || command.starts_with("view:")
        || command.starts_with("sort:")
        || command.starts_with("sort_direction:")
    {
        run_in_context_panel(app, menu.origin_panel_is_left, |app| {
            crate::app::operations::context_menu::background::handle_view_command(app, command);
        });
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;
    use std::path::PathBuf;

    #[test]
    fn background_target_uses_captured_directory_not_a_previous_selection() {
        let mut menu = ContextMenuState::new();
        menu.open(
            egui::Pos2::ZERO,
            None,
            vec![PathBuf::from(r"D:\Folder")],
            true,
        );
        menu.target_paths = vec![PathBuf::from(r"C:\Other\file.txt")];
        assert_eq!(folder_target(&menu), Some(Path::new(r"D:\Folder")));
    }

    #[test]
    fn selected_item_is_not_a_folder_background_target() {
        let mut menu = ContextMenuState::new();
        menu.open(
            egui::Pos2::ZERO,
            None,
            vec![PathBuf::from(r"C:\Folder\file.txt")],
            false,
        );
        assert_eq!(folder_target(&menu), None);
    }
}
