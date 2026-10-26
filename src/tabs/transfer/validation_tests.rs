use super::*;
use crate::domain::file_entry::ViewMode;
use crate::tabs::session::{
    SessionColumnWidths, SessionDualPanelTabs, SessionPanelTab, SessionPanelTabSet,
};

fn split_transfer() -> TabTransfer {
    let manager = TabManager::new_at_path("C:\\");
    let mut transfer = TabTransfer::from_tab(manager.active());
    let panel = |id, path: &str| SessionPanelTab {
        pinned: false,
        id,
        path: path.to_owned(),
        view_mode: "grid".to_owned(),
        sort_mode: "name".to_owned(),
        sort_descending: false,
        folders_position: "first".to_owned(),
        group_mode: "none".to_owned(),
        group_descending: false,
        active_tag_filter: None,
        column_widths: SessionColumnWidths {
            values: [120.0; 12],
        },
    };
    transfer.session.tabs[0].dual_panel_enabled = true;
    transfer.session.tabs[0].dual_panel_tabs = Some(SessionDualPanelTabs {
        left: SessionPanelTabSet {
            active_tab: 0,
            tabs: vec![panel(0, "C:\\")],
        },
        right: SessionPanelTabSet {
            active_tab: 0,
            tabs: vec![panel(1, "D:\\")],
        },
    });
    transfer.panels = Some((
        vec![transfer.browser.clone()],
        vec![BrowserState::capture(
            &NavigationHistory::new("D:\\".to_owned()),
            "",
            0.0,
            0.0,
        )],
    ));
    transfer
}

#[test]
fn transfer_rejects_session_repairs_that_would_discard_panel_tabs() {
    let mut transfer = split_transfer();
    assert!(TabTransfer::from_json(&serde_json::to_string(&transfer).unwrap()).is_some());
    transfer.session.tabs[0]
        .dual_panel_tabs
        .as_mut()
        .unwrap()
        .left
        .tabs
        .clear();
    let value = serde_json::to_string(&transfer).unwrap();
    // Persisted sessions repair this damage by dropping split mode, but a
    // transfer must not acknowledge that lossy repair as a successful move.
    assert!(TabManagerSession::from_json(&transfer.session.to_json().unwrap()).is_some());
    assert!(TabTransfer::from_json(&value).is_none());
}

#[test]
fn transfer_rejects_missing_browser_state_and_invalid_history() {
    let mut transfer = split_transfer();
    transfer.panels.as_mut().unwrap().1.clear();
    assert!(TabTransfer::from_json(&serde_json::to_string(&transfer).unwrap()).is_none());
    let mut transfer = split_transfer();
    transfer.browser.history_index = usize::MAX;
    assert!(TabTransfer::from_json(&serde_json::to_string(&transfer).unwrap()).is_none());
}

#[test]
fn changed_startup_location_must_not_acknowledge_the_move() {
    let mut manager = TabManager::new_at_path("C:\\");
    let transfer = TabTransfer::from_tab(manager.active());
    assert!(transfer.locations_match(&manager));
    manager.active_mut().view_mode = ViewMode::List;
    assert!(transfer.locations_match(&manager));
    manager.active_mut().navigate_to("D:\\");
    assert!(!transfer.locations_match(&manager));

    let transfer = split_transfer();
    let expected = &transfer.session.tabs[0]
        .dual_panel_tabs
        .as_ref()
        .unwrap()
        .right;
    let mut actual = expected.clone();
    assert!(panel_locations_match(expected, &actual));
    actual.tabs[0].path = "E:\\".to_owned();
    assert!(!panel_locations_match(expected, &actual));
}
