use super::*;
use crate::app::dual_panel::{ActivePanel, PanelListColumnWidths};
use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{FoldersPosition, GroupMode, SortMode, ViewMode};
use crate::tabs::{DualPanelTabs, TabManager};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Instant;

fn snapshot(path: &str) -> PanelSnapshot {
    PanelSnapshot {
        path: path.to_owned(),
        current_folder_modified_hint: Some((PathBuf::from(path), 100)),
        current_folder_created_hint: Some((PathBuf::from(path), 50)),
        path_input: path.to_owned(),
        is_computer_view: false,
        is_recycle_bin_view: false,
        navigation: NavigationHistory::new(path.to_owned()),
        items: Default::default(),
        all_items: Default::default(),
        items_revision: 0,
        items_snapshot_compact: false,
        total_items: 0,
        is_loading_folder: false,
        folder_load_error: None,
        selected_item: None,
        selected_file: None,
        multi_selection: Default::default(),
        selection_anchor: None,
        rectangle_selection_state: None,
        view_mode: ViewMode::List,
        sort_mode: SortMode::Name,
        sort_descending: false,
        folders_position: FoldersPosition::First,
        group_mode: GroupMode::None,
        group_descending: false,
        group_projection: Default::default(),
        collapsed_groups_by_context: Default::default(),
        current_folder_locked: false,
        list_column_widths: PanelListColumnWidths::from_layout(&Default::default()),
        pending_list_column_autofit: false,
        miller_columns: Default::default(),
        scroll_offset_y: 0.0,
        scroll_offset_x: 0.0,
        last_scroll_offset: 0.0,
        last_scroll_time: Instant::now(),
        scroll_to_selected: false,
        visible_index_range: None,
        visible_paths_cache: Default::default(),
        visible_group_paths: Vec::new(),
        visible_range_cached: None,
        search_query: String::new(),
        active_tag_filter: None,
        selected_thumbnail: None,
        selected_metadata: None,
        selected_gif: None,
        loaded_path: path.to_owned(),
        renaming_state: None,
        focus_rename: false,
        generation: 0,
        current_generation: Arc::new(AtomicUsize::new(0)),
        folder_load_generation: Arc::new(AtomicUsize::new(0)),
        pending_all_items_clear: false,
        hold_visible_items_until_load_complete: false,
        pending_items_rebuild: false,
        pending_items_count: 0,
        inactive_final_items_rebuild_pending: false,
        loading_started_at: Instant::now(),
        last_items_rebuild: Instant::now(),
        stale_items_snapshot: None,
    }
}

#[test]
fn resolved_dates_update_all_matching_outer_and_inner_tabs_after_focus_changes() {
    let folder = r"C:\Folder";
    let other = r"D:\Other";
    let mut tabs = TabManager::new_at_path(folder);
    tabs.active_mut().current_folder_modified_hint = Some((PathBuf::from(folder), 100));
    tabs.active_mut().dual_panel_inactive_state = Some(snapshot(folder));
    let mut panels = DualPanelTabs::new(snapshot(folder), snapshot(other));
    panels.add_tab(ActivePanel::Left, snapshot(r"c:\FOLDER"));
    panels.add_tab(ActivePanel::Right, snapshot(folder));
    tabs.active_mut().dual_panel_tabs = Some(panels);
    tabs.new_tab_at(other);
    let mut inactive = Some(snapshot(folder));
    let mut updated = 0;
    visit_stored(&mut inactive, &mut tabs, |owner, modified, created| {
        if apply_hints(
            owner,
            modified,
            created,
            Path::new(r"\\?\C:\Folder"),
            300,
            Some(200),
        ) {
            updated += 1;
        }
    });
    assert_eq!(updated, 6);
    assert_eq!(tabs.active().path, other);
    assert!(tabs.active().current_folder_modified_hint.is_none());
    visit_stored(&mut inactive, &mut tabs, |owner, modified, created| {
        if normalize_tag_path_key(Path::new(owner)) == normalize_tag_path_key(Path::new(folder)) {
            assert_eq!(modified, &Some((PathBuf::from(owner), 300)));
            assert_eq!(created, &Some((PathBuf::from(owner), 200)));
        }
    });
    assert_eq!(
        tabs.tabs[0].dual_panel_tabs.as_ref().unwrap().right.tabs[0]
            .snapshot
            .current_folder_modified_hint,
        Some((PathBuf::from(other), 100))
    );
}

#[test]
fn alias_updates_preserve_owner_spelling_and_partial_values() {
    let owner = r"C:\Folder";
    let mut modified = Some((PathBuf::from(owner), 100));
    let mut created = Some((PathBuf::from(owner), 50));
    assert!(apply_hints(
        owner,
        &mut modified,
        &mut created,
        Path::new(r"\\?\c:\FOLDER"),
        300,
        None
    ));
    assert_eq!(modified, Some((PathBuf::from(owner), 300)));
    assert_eq!(created, Some((PathBuf::from(owner), 50)));
    assert!(!apply_hints(
        r"C:\Folder2",
        &mut modified,
        &mut created,
        Path::new(owner),
        400,
        Some(500)
    ));
    assert_eq!(modified.unwrap().1, 300);
}
