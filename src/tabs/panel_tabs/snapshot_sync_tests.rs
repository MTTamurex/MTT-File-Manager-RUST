use super::*;
use crate::app::dual_panel::PanelListColumnWidths;
use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{FoldersPosition, GroupMode, SortMode, ViewMode};
use crate::tabs::{TabManager, TabManagerSession};
use std::sync::{atomic::AtomicUsize, Arc};
use std::time::Instant;

fn snapshot(path: &str) -> PanelSnapshot {
    PanelSnapshot {
        path: path.to_owned(),
        current_folder_modified_hint: None,
        current_folder_created_hint: None,
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
        scroll_offset_y: 120.0,
        scroll_offset_x: 0.0,
        last_scroll_offset: 120.0,
        last_scroll_time: Instant::now(),
        scroll_to_selected: false,
        visible_index_range: None,
        visible_paths_cache: Default::default(),
        visible_group_paths: Vec::new(),
        visible_range_cached: None,
        search_query: "file".to_owned(),
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
fn batch_close_panel_tabs_keeps_target_focus_when_selected_tab_is_closed() {
    use crate::tabs::actions::{close_candidates, TabAction};

    for panel in [ActivePanel::Left, ActivePanel::Right] {
        for action in [TabAction::CloseLeft, TabAction::CloseRight] {
            for pinned in [false, true] {
                for target in 0..5 {
                    for selected in 0..5 {
                        let mut tabs = DualPanelTabs::new(snapshot("C:\\"), snapshot("D:\\"));
                        for _ in 1..5 {
                            tabs.add_tab(panel, snapshot("C:\\"));
                        }
                        let set = tabs.panel_mut(panel);
                        set.tabs[0].pinned = pinned;
                        set.select(selected);
                        let candidates = close_candidates(&set.tabs, target, action, |t| t.pinned);
                        let closing_active = candidates.contains(&selected);
                        let expected_active =
                            set.tabs[if closing_active { target } else { selected }].id;
                        let closed_ids: Vec<_> =
                            candidates.iter().map(|&i| set.tabs[i].id).collect();
                        let target_id = set.tabs[target].id;
                        if closing_active {
                            assert!(set.select(target));
                        }
                        for index in candidates {
                            assert_eq!(set.close(index), Some(false));
                        }
                        assert_eq!(set.active().unwrap().id, expected_active);
                        assert!(set.tabs.iter().any(|t| t.id == target_id));
                        assert!(set.tabs.iter().all(|t| !closed_ids.contains(&t.id)));
                        if pinned {
                            assert!(set.tabs[0].pinned);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn single_panel_navigation_preserves_four_pinned_tabs_and_resumes_the_selected_one() {
    let paths = ["C:\\", "D:\\", "E:\\", "F:\\"];
    for focused in [ActivePanel::Left, ActivePanel::Right] {
        for selected in 0..paths.len() {
            let mut tabs = DualPanelTabs::new(snapshot(paths[0]), snapshot(paths[0]));
            for path in &paths[1..] {
                tabs.add_tab(focused, snapshot(path));
            }
            for tab in &mut tabs.panel_mut(focused).tabs {
                tab.pinned = true;
            }
            tabs.panel_mut(focused).select(selected);
            let selected_id = tabs.panel(focused).active().unwrap().id;
            let other_id = tabs.panel(focused.other()).active().unwrap().id;
            let original = tabs.panel(focused).active().unwrap().snapshot.clone();

            // Disable dual mode, navigate elsewhere, then sync again while saving
            // preferences and immediately before restoring the split layout.
            tabs.panel_mut(focused)
                .sync_active_snapshot(original.clone(), true);
            for path in ["Z:\\Folder", "Z:\\Other Folder", "Z:\\Other Folder"] {
                tabs.panel_mut(focused)
                    .sync_active_snapshot(snapshot(path), false);
            }

            // Saving while dual mode is off keeps the single-pane destination
            // separate from the dormant pinned tabs in the persisted session.
            let mut manager = TabManager::new_at_path("Z:\\Other Folder");
            manager.active_mut().dual_panel_tabs = Some(tabs.clone());
            manager.active_mut().dual_panel_active = focused;
            let json = TabManagerSession::from_tab_manager(&manager)
                .to_json()
                .unwrap();
            let session = TabManagerSession::from_json(&json).unwrap();
            let outer = &session.tabs[0];
            assert!(!outer.dual_panel_enabled);
            assert_eq!(outer.path, "Z:\\Other Folder");
            let saved_tabs = outer.dual_panel_tabs.as_ref().unwrap();
            let saved_panel = match focused {
                ActivePanel::Left => &saved_tabs.left,
                ActivePanel::Right => &saved_tabs.right,
            };
            assert_eq!(saved_panel.active_tab, selected);
            assert_eq!(saved_panel.tabs[selected].path, paths[selected]);
            assert!(saved_panel.tabs.iter().all(|tab| tab.pinned));

            let resumed = tabs.panel(focused).pinned_snapshot_for_resume().unwrap();
            assert_eq!(resumed.path, paths[selected]);
            assert_eq!(resumed.navigation.current_path(), Some(&original.path));
            assert_eq!(resumed.search_query, original.search_query);
            assert_eq!(resumed.scroll_offset_y, original.scroll_offset_y);
            tabs.panel_mut(focused).sync_active_snapshot(resumed, true);

            assert_eq!(tabs.panel(focused).active_tab, selected);
            assert_eq!(tabs.panel(focused).active().unwrap().id, selected_id);
            assert_eq!(tabs.panel(focused).tabs.len(), paths.len());
            for (tab, path) in tabs.panel(focused).tabs.iter().zip(paths) {
                assert!(tab.pinned);
                assert_eq!(tab.snapshot.path, path);
            }
            let other = tabs.panel(focused.other());
            assert_eq!(other.tabs.len(), 1);
            assert_eq!(other.active().unwrap().id, other_id);
            assert!(!other.active().unwrap().pinned);
            assert_eq!(other.active().unwrap().snapshot.path, paths[0]);
        }
    }
}

#[test]
fn unpinned_tabs_still_follow_single_panel_navigation() {
    for focused in [ActivePanel::Left, ActivePanel::Right] {
        let mut tabs = DualPanelTabs::new(snapshot("C:\\"), snapshot("D:\\"));
        tabs.panel_mut(focused)
            .sync_active_snapshot(snapshot("Z:\\Folder"), false);
        let active = tabs.panel(focused).active().unwrap();
        assert_eq!(active.snapshot.path, "Z:\\Folder");
        assert!(tabs.panel(focused).pinned_snapshot_for_resume().is_none());
    }
}

#[test]
fn folder_timestamps_survive_panel_tab_and_outer_tab_round_trips() {
    for focused in [ActivePanel::Left, ActivePanel::Right] {
        let mut first = snapshot(r"C:\Folder");
        first.current_folder_modified_hint = Some((PathBuf::from(&first.path), 100));
        first.current_folder_created_hint = Some((PathBuf::from(&first.path), 50));
        let mut second = snapshot(r"D:\Folder");
        second.current_folder_modified_hint = Some((PathBuf::from(&second.path), 200));
        let mut tabs = DualPanelTabs::new(first.clone(), first.clone());
        tabs.add_tab(focused, second.clone());
        let mut manager = TabManager::new_at_path(r"C:\Folder");
        manager.active_mut().dual_panel_tabs = Some(tabs);
        assert!(manager.new_tab_at(r"E:\Folder"));
        manager.active_tab = 0;
        let tabs = manager.active_mut().dual_panel_tabs.as_mut().unwrap();
        for index in [0, 1, 0] {
            tabs.panel_mut(focused).select(index);
            let mut restored = tabs.panel(focused).active().unwrap().snapshot.clone();
            restored.compact_for_storage();
            restored.restore_from_storage();
            let expected = if index == 0 { &first } else { &second };
            assert_eq!(restored.path, expected.path);
            assert_eq!(
                restored.current_folder_modified_hint,
                expected.current_folder_modified_hint
            );
            assert_eq!(
                restored.current_folder_created_hint,
                expected.current_folder_created_hint
            );
            tabs.panel_mut(focused).sync_active_snapshot(restored, true);
            let other = &tabs.panel(focused.other()).active().unwrap().snapshot;
            assert_eq!(
                other.current_folder_modified_hint,
                first.current_folder_modified_hint
            );
            assert_eq!(
                other.current_folder_created_hint,
                first.current_folder_created_hint
            );
        }
    }
}

#[test]
fn redirecting_panel_snapshot_clears_previous_folder_timestamps() {
    let mut panel = snapshot(r"C:\Folder");
    panel.current_folder_modified_hint = Some((PathBuf::from(&panel.path), 100));
    panel.current_folder_created_hint = Some((PathBuf::from(&panel.path), 50));
    panel.navigation.navigate_to(r"D:\Folder".to_owned());
    panel.redirect_to_history_current();
    assert_eq!(panel.path, r"D:\Folder");
    assert!(panel.current_folder_modified_hint.is_none());
    assert!(panel.current_folder_created_hint.is_none());
}

#[test]
fn live_dual_panel_sync_still_updates_pinned_listing_state() {
    let mut tabs = DualPanelTabs::new(snapshot("C:\\"), snapshot("D:\\"));
    tabs.left.active_mut().unwrap().pinned = true;
    let mut updated = snapshot("C:\\");
    updated.items_revision = 10;
    updated.scroll_offset_y = 240.0;
    tabs.left.sync_active_snapshot(updated, true);
    let restored = tabs.left.pinned_snapshot_for_resume().unwrap();
    assert_eq!(restored.items_revision, 10);
    assert_eq!(restored.scroll_offset_y, 240.0);
}
