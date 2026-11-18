use crate::app::ImageViewerApp;

#[derive(Default)]
pub(super) struct DeferredActions {
    pub reorder: Option<(usize, usize)>,
    pub navigate: Option<Option<String>>,
    pub new_tab_path: Option<String>,
    pub context_menu_path: Option<String>,
    pub unpin_path: Option<String>,
    pub pin_path: Option<String>,
    pub pin_current_path: Option<String>,
    pub open_group_id: Option<i64>,
    pub delete_group_id: Option<i64>,
    pub cancel_drag_on_bar: bool,
    pub drop_to_recycle_bin: bool,
}

pub(super) fn apply(
    app: &mut ImageViewerApp,
    root_ui: &eframe::egui::Ui,
    actions: DeferredActions,
) {
    if actions.drop_to_recycle_bin {
        app.complete_item_drag_to_recycle_bin();
    }
    if actions.cancel_drag_on_bar {
        app.cancel_item_drag();
    }
    if let Some(target) = actions.navigate {
        match target {
            Some(path) => app.navigate_to(&path),
            None => app.navigate_to_recycle_bin(),
        }
    }
    if let Some(path) = actions.new_tab_path {
        app.open_location_in_new_tab(app.dual_panel_active, &path);
    }
    if let Some(group_id) = actions.open_group_id {
        app.open_folder_group(group_id);
    }
    if let Some(group_id) = actions.delete_group_id {
        app.delete_folder_group(group_id);
    }
    if let Some((from, to)) = actions.reorder {
        app.reorder_quick_access_item(from, to);
    }
    if let Some(path) = actions.unpin_path {
        app.unpin_folder(&path);
    }
    if let Some(path) = actions.pin_path {
        app.pin_folder(&path);
    }
    if let Some(path) = actions.pin_current_path {
        app.pin_folder(&path);
    }
    if let Some(path) = actions.context_menu_path {
        let path_buf = std::path::PathBuf::from(path);
        let position = root_ui
            .ctx()
            .input(|input| input.pointer.hover_pos().unwrap_or_default());
        app.context_menu
            .open(position, None, vec![path_buf.clone()], false);
        app.context_menu.primary_is_directory = Some(true);
        app.populate_context_menu(root_ui.ctx(), &[path_buf], false, None);
    }
}
