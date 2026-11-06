use super::*;

#[test]
fn geometry_round_trips_through_preferences() {
    let db = AppStateDb::new_in_memory().unwrap();
    assert_eq!(SettingsWindowGeometry::load(&db), None);
    let rect = egui::Rect::from_min_size(egui::pos2(125.0, 85.0), egui::vec2(800.0, 600.0));
    let geometry = SettingsWindowGeometry::from_rect(rect).unwrap();
    db.set_preferences_batch(&[geometry.preference().unwrap()])
        .unwrap();
    assert_eq!(SettingsWindowGeometry::load(&db), Some(geometry));
    assert_eq!(SettingsWindowGeometry::load(&db).unwrap().rect(), rect);
    let layout = crate::app::init_state_builders::build_layout_state(
        &db, 1280.0, 720.0, false, 200.0, 300.0, 0.5, None, None,
    );
    assert_eq!(layout.settings_window_geometry, Some(geometry));
}

#[test]
fn invalid_saved_geometry_is_ignored() {
    let db = AppStateDb::new_in_memory().unwrap();
    for value in [
        "not json",
        "{}",
        r#"{"x":0,"y":0,"width":0,"height":500}"#,
        r#"{"x":0,"y":0,"width":800,"height":-1}"#,
        r#"{"x":null,"y":0,"width":800,"height":500}"#,
        r#"{"x":3e38,"y":0,"width":3e38,"height":500}"#,
    ] {
        db.set_preference(PREFERENCE_KEY, value).unwrap();
        assert_eq!(SettingsWindowGeometry::load(&db), None, "{value}");
    }
    assert!(SettingsWindowGeometry::from_rect(egui::Rect::NOTHING).is_none());
}
