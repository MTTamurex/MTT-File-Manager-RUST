use super::*;
use crate::app::dual_panel::{ActivePanel, PanelListColumnWidths};
use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::tabs::DualPanelTabs;
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Instant;

fn entries(path: &str) -> Arc<Vec<FileEntry>> {
    Arc::new(vec![FileEntry {
        path: PathBuf::from(format!(r"{}\file.txt", path)),
        name: "file.txt".to_string(),
        is_dir: false,
        size: 1,
        modified: 0,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden: false,
        recycle_bin: None,
    }])
}

fn tab(id: usize, path: &str) -> TabState {
    let mut tab = TabState::new_at_path(id, path);
    tab.all_items = entries(path);
    tab.items_snapshot_compact = true;
    tab.total_items = 1;
    tab.selected_item = Some(0);
    tab.selected_file = tab.all_items.first().cloned();
    tab.generation = super::super::folder_loading::allocate_folder_load_generation();
    tab
}

fn snapshot(path: &str) -> PanelSnapshot {
    let tab = tab(0, path);
    PanelSnapshot {
        path: path.to_string(),
        path_input: path.to_string(),
        is_computer_view: false,
        is_recycle_bin_view: false,
        navigation: NavigationHistory::new(path.to_string()),
        items: tab.all_items.clone(),
        all_items: tab.all_items,
        items_revision: 0,
        items_snapshot_compact: false,
        total_items: 1,
        is_loading_folder: true,
        folder_load_error: None,
        selected_item: Some(0),
        selected_file: tab.selected_file,
        multi_selection: Default::default(),
        selection_anchor: Some(0),
        rectangle_selection_state: None,
        view_mode: tab.view_mode,
        sort_mode: tab.sort_mode,
        sort_descending: false,
        folders_position: tab.folders_position,
        group_mode: tab.group_mode,
        group_descending: false,
        group_projection: Default::default(),
        collapsed_groups_by_context: Default::default(),
        current_folder_locked: false,
        list_column_widths: PanelListColumnWidths::from_layout(&Default::default()),
        pending_list_column_autofit: false,
        miller_columns: Default::default(),
        scroll_offset_y: 12.0,
        scroll_offset_x: 0.0,
        scroll_to_selected: true,
        visible_index_range: Some((0, 1)),
        visible_paths_cache: Default::default(),
        visible_group_paths: Vec::new(),
        visible_range_cached: None,
        search_query: "file".to_string(),
        active_tag_filter: None,
        selected_thumbnail: None,
        selected_metadata: None,
        selected_gif: None,
        loaded_path: path.to_string(),
        renaming_state: None,
        focus_rename: false,
        generation: tab.generation,
        current_generation: Arc::new(AtomicUsize::new(tab.generation)),
        folder_load_generation: Arc::new(AtomicUsize::new(tab.generation)),
        pending_all_items_clear: false,
        hold_visible_items_until_load_complete: true,
        pending_items_rebuild: true,
        pending_items_count: 1,
        inactive_final_items_rebuild_pending: true,
        loading_started_at: Instant::now(),
        last_items_rebuild: Instant::now(),
        stale_items_snapshot: None,
    }
}

#[test]
fn mono_panel_tabs_keep_identity_and_drop_compacted_removed_drive_contents() {
    let mut removed_tab = tab(11, r"r:\Folder");
    let generation = removed_tab.generation;
    let mut unaffected = tab(12, r"S:\Folder");
    let unaffected_items = unaffected.all_items.clone();
    let removed = vec![r"R:\".to_string()];
    redirect_stored_tab(&mut removed_tab, &removed);
    redirect_stored_tab(&mut unaffected, &removed);
    assert_eq!(removed_tab.id, 11);
    assert_eq!(removed_tab.path, COMPUTER_VIEW_ID);
    assert_eq!(removed_tab.title, COMPUTER_VIEW_ID);
    assert_eq!(removed_tab.path_input, COMPUTER_VIEW_ID);
    assert!(removed_tab.is_computer_view);
    assert!(removed_tab.restore_items_snapshot().is_empty());
    assert_eq!(removed_tab.total_items, 0);
    assert!(removed_tab.selected_file.is_none());
    assert_ne!(removed_tab.generation, generation);
    assert_eq!(unaffected.path, r"S:\Folder");
    assert!(Arc::ptr_eq(&unaffected.all_items, &unaffected_items));
}

#[test]
fn dual_panel_removal_covers_both_sides_and_background_inner_tabs() {
    for focused in [ActivePanel::Left, ActivePanel::Right] {
        let mut workspace = tab(31, r"S:\Folder");
        workspace.dual_panel_enabled = true;
        workspace.dual_panel_active = focused;
        workspace.dual_panel_inactive_state = Some(snapshot(r"R:\Folder"));
        let token = workspace
            .dual_panel_inactive_state
            .as_ref()
            .unwrap()
            .folder_load_generation
            .clone();
        let old_generation = token.load(Ordering::Relaxed);
        let mut panels = DualPanelTabs::new(snapshot(r"R:\Left"), snapshot(r"S:\Right"));
        panels
            .add_tab(ActivePanel::Right, snapshot(r"R:\Background"))
            .unwrap();
        panels.right.active_tab = 0;
        let right_id = panels.right.tabs[1].id;
        let count = panels.count();
        workspace.dual_panel_tabs = Some(panels);
        redirect_stored_tab(&mut workspace, &[r"R:\".to_string()]);
        assert_eq!(workspace.path, r"S:\Folder");
        assert_eq!(workspace.dual_panel_active, focused);
        let inactive = workspace.dual_panel_inactive_state.as_ref().unwrap();
        assert_eq!(inactive.path, COMPUTER_VIEW_ID);
        assert!(inactive.items.is_empty() && inactive.all_items.is_empty());
        assert!(!inactive.is_loading_folder && !inactive.pending_items_rebuild);
        assert!(!inactive.hold_visible_items_until_load_complete);
        assert!(inactive.loaded_path.is_empty());
        assert_ne!(token.load(Ordering::Relaxed), old_generation);
        let panels = workspace.dual_panel_tabs.as_ref().unwrap();
        assert_eq!(panels.count(), count);
        assert_eq!(panels.right.active_tab, 0);
        assert_eq!(panels.left.tabs[0].snapshot.path, COMPUTER_VIEW_ID);
        assert_eq!(panels.right.tabs[0].snapshot.path, r"S:\Right");
        assert_eq!(panels.right.tabs[1].id, right_id);
        assert_eq!(panels.right.tabs[1].snapshot.path, COMPUTER_VIEW_ID);
        assert!(panels.right.tabs[1].snapshot.selected_file.is_none());
    }
}

#[test]
fn removed_roots_ignore_case_labels_and_cloud_reclassification() {
    let old = vec![
        (r"C:\".to_string(), "Old label".to_string()),
        (r"R:\".to_string(), String::new()),
    ];
    let current = vec![("c:/".to_string(), "New label".to_string())];
    assert_eq!(
        removed_drive_roots(&old, &current, &HashSet::new()),
        vec![r"R:\"]
    );
    assert!(
        removed_drive_roots(&old, &current, &HashSet::from([r"r:\Mount".to_string()])).is_empty()
    );
    assert!(!is_on_removed_drive(r"S:\R\Folder", &[r"R:\".to_string()]));
    assert!(!is_on_removed_drive(
        COMPUTER_VIEW_ID,
        &[r"R:\".to_string()]
    ));
    assert!(is_on_removed_drive(r"\\?\r:\Folder", &[r"R:\".to_string()]));
}
