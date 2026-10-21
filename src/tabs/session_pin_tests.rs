use super::{TabManager, TabManagerSession};

#[test]
fn session_restores_pin_order_and_active_identity() {
    let mut manager = TabManager::new();
    manager.new_tab_at("C:\\");
    manager.new_tab_at("D:\\");
    manager.toggle_pin(1);
    let session = TabManagerSession::from_tab_manager(&manager);
    let restored = TabManagerSession::from_json(&session.to_json().unwrap())
        .unwrap()
        .into_tab_manager();
    assert!(restored.tabs[0].pinned);
    assert_eq!(restored.tabs[0].path, "C:\\");
    assert_eq!(restored.active().path, "D:\\");
    assert!(!restored.active().pinned);
}

#[test]
fn older_sessions_default_to_unpinned_without_losing_tabs() {
    let manager = TabManager::new();
    let serialized = TabManagerSession::from_tab_manager(&manager)
        .to_json()
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    value["tabs"][0].as_object_mut().unwrap().remove("pinned");
    let restored = TabManagerSession::from_json(&value.to_string())
        .unwrap()
        .into_tab_manager();
    assert_eq!(restored.tabs.len(), 1);
    assert!(!restored.tabs[0].pinned);
}
