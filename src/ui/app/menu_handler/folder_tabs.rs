use super::{context_origin_panel, primary_context_target};
use crate::app::ImageViewerApp;
use crate::application::context_menu::ContextMenuState;

fn destinations(menu: &ContextMenuState) -> Vec<String> {
    if menu.target_paths.len() > 1 {
        return menu
            .folder_targets
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
    }
    let Some((path, directory)) = primary_context_target(menu) else {
        return Vec::new();
    };
    let target = if directory {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    vec![target.to_string_lossy().into_owned()]
}

pub(super) fn open(app: &mut ImageViewerApp, menu: &ContextMenuState, ctrl: bool) {
    let paths = destinations(menu);
    if paths.is_empty() {
        return;
    }
    let panel = context_origin_panel(menu.origin_panel_is_left, app.dual_panel_active);
    let created = app.open_locations_in_new_tabs(panel, &paths, ctrl);
    if created
        && menu.origin == crate::application::context_menu::ContextMenuOrigin::GlobalSearch
        && app.tab_preferences.should_activate_new_tab(ctrl)
    {
        app.close_global_search();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_selection_only_opens_captured_folders_in_order() {
        let mut menu = ContextMenuState::default();
        menu.open(
            eframe::egui::Pos2::ZERO,
            None,
            vec!["file.txt".into(), "Folder A".into(), "Folder B".into()],
            false,
        );
        menu.folder_targets = vec!["Folder A".into(), "Folder B".into()];
        menu.primary_is_directory = Some(false);
        assert_eq!(destinations(&menu), ["Folder A", "Folder B"]);
    }

    #[test]
    fn multiple_files_do_not_open_their_parent_folders() {
        let mut menu = ContextMenuState::default();
        menu.open(
            eframe::egui::Pos2::ZERO,
            None,
            vec!["file.txt".into(), "other.txt".into()],
            false,
        );
        assert!(destinations(&menu).is_empty());
    }

    #[test]
    fn single_file_keeps_parent_folder_behavior() {
        let mut menu = ContextMenuState::default();
        menu.open(
            eframe::egui::Pos2::ZERO,
            None,
            vec![r"C:\Folder\file.txt".into()],
            false,
        );
        menu.primary_is_directory = Some(false);
        assert_eq!(destinations(&menu), [r"C:\Folder"]);
    }
}
