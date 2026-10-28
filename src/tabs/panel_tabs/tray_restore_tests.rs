use super::{snapshot_sync_tests::snapshot, *};
use crate::app::init_workers::consistency_probe_worker::{
    spawn_consistency_probe_worker, ConsistencyProbeMode, ConsistencyProbeRequest,
};
use crate::domain::special_paths::tag_view_path;
use crate::infrastructure::windows::system_tray::model::{
    Entry, LocationKind, Snapshot, Target, Workspace,
};
use crate::tabs::{TabManager, TabManagerSession};
use std::path::PathBuf;
use std::time::Duration;

fn tag_snapshot(id: i64) -> PanelSnapshot {
    let mut state = snapshot(&tag_view_path(id));
    state.active_tag_filter = Some(id);
    state.search_query.clear();
    state
}

#[test]
fn four_pinned_tag_tabs_survive_tray_selection_and_a_delayed_liveness_probe() {
    let (requests, results) = spawn_consistency_probe_worker(eframe::egui::Context::default());
    for selected in 0..4 {
        let mut panels = DualPanelTabs::new(tag_snapshot(1), tag_snapshot(5));
        for id in 2..=4 {
            panels
                .add_background_tab(ActivePanel::Left, tag_snapshot(id))
                .unwrap();
        }
        for tab in &mut panels.left.tabs {
            tab.pinned = true;
        }
        panels.left.select(selected);
        let original_ids = panels
            .left
            .tabs
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>();
        let target_id = panels.left.active().unwrap().id;
        let path = panels.left.active().unwrap().snapshot.path.clone();
        let mut manager = TabManager::new_at_path(&tag_view_path(5));
        manager.active_mut().dual_panel_enabled = true;
        manager.active_mut().dual_panel_active = ActivePanel::Right;
        manager.active_mut().dual_panel_tabs = Some(panels);

        // Reproduce an obsolete request queued for the inactive left tag while
        // the right panel was focused. Receive it only after tray activation.
        requests
            .send(ConsistencyProbeRequest {
                path: PathBuf::from(&path),
                is_onedrive: false,
                ui_signature: 0,
                show_hidden_files: false,
                mode: ConsistencyProbeMode::PathLiveness,
                modified_threshold_secs: 0,
                folder_cover_states: Vec::new(),
            })
            .unwrap();
        let workspace = manager.active();
        let panels = workspace.dual_panel_tabs.as_ref().unwrap();
        let menu = Snapshot {
            workspaces: vec![Workspace {
                tab: Entry {
                    id: workspace.id,
                    path: workspace.path.clone(),
                    icon: LocationKind::Tag,
                },
                panels: Some([ActivePanel::Left, ActivePanel::Right].map(|panel| {
                    panels
                        .panel(panel)
                        .tabs
                        .iter()
                        .map(|tab| Entry {
                            id: tab.id,
                            path: tab.snapshot.path.clone(),
                            icon: LocationKind::Tag,
                        })
                        .collect()
                })),
            }],
            active_path: tag_view_path(5),
            active_icon: LocationKind::Tag,
            hidden_at: None,
        };
        let resolved = menu
            .resolve(&Target {
                workspace: workspace.id,
                panel: Some((false, target_id)),
            })
            .unwrap();
        assert_eq!(resolved.workspace_index, manager.active_tab);
        assert_eq!(resolved.panel, Some((false, selected)));
        let workspace = manager.active_mut();
        let panels = workspace.dual_panel_tabs.as_mut().unwrap();
        panels.left.select(resolved.panel.unwrap().1);
        let restored = panels.left.pinned_snapshot_for_resume().unwrap();
        workspace.path = restored.path;
        workspace.path_input = restored.path_input;
        workspace.active_tag_filter = restored.active_tag_filter;
        workspace.navigation = restored.navigation;
        workspace.dual_panel_active = ActivePanel::Left;

        let delayed = results.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(delayed.path, PathBuf::from(&path));
        assert!(
            !delayed.path_vanished,
            "a tag must never trigger filesystem recovery"
        );
        let panels = manager.active().dual_panel_tabs.as_ref().unwrap();
        assert_eq!(manager.active().path, path);
        assert_eq!(
            panels
                .left
                .tabs
                .iter()
                .map(|tab| tab.id)
                .collect::<Vec<_>>(),
            original_ids
        );
        for (index, tab) in panels.left.tabs.iter().enumerate() {
            assert!(tab.pinned);
            assert_eq!(tab.snapshot.path, tag_view_path(index as i64 + 1));
            assert_eq!(tab.snapshot.active_tag_filter, Some(index as i64 + 1));
            assert_eq!(
                tab.snapshot.navigation.current_path(),
                Some(&tab.snapshot.path)
            );
        }
        assert_eq!(panels.right.tabs.len(), 1);
        assert_eq!(
            panels.right.active().unwrap().snapshot.path,
            tag_view_path(5)
        );
        let session = TabManagerSession::from_json(
            &TabManagerSession::from_tab_manager(&manager)
                .to_json()
                .unwrap(),
        )
        .unwrap();
        let saved = session.tabs[0].dual_panel_tabs.as_ref().unwrap();
        assert_eq!(saved.left.active_tab, selected);
        for (index, tab) in saved.left.tabs.iter().enumerate() {
            assert!(tab.pinned);
            assert_eq!(tab.path, tag_view_path(index as i64 + 1));
        }
        assert_eq!(saved.right.tabs[0].path, tag_view_path(5));
    }
}
