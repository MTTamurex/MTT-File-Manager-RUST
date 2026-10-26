use super::init_preferences::StartupPreferences;
use super::startup_location::StartupLocation;
use crate::domain::date_format::DateFormat;
use crate::infrastructure::app_state_db::AppStateDb;

fn saved_workspace_preferences() -> StartupPreferences {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    db.set_preference("dual_panel_enabled", "true").unwrap();
    db.set_preference("dual_panel_inactive_path", r"D:\Documents")
        .unwrap();
    db.set_preference("active_tag_filter", "1").unwrap();
    let mut tabs = crate::tabs::TabManager::new();
    tabs.new_tab_at(r"D:\Videos");
    let session = crate::tabs::TabManagerSession::from_tab_manager(&tabs);
    db.set_preference("tab_session", &session.to_json().unwrap())
        .unwrap();
    StartupPreferences::load(&db)
}

#[test]
fn fixed_location_skips_both_saved_session_and_legacy_panels() {
    let temp = tempfile::tempdir().unwrap();
    for location in [
        StartupLocation::Computer,
        StartupLocation::Documents,
        StartupLocation::UserProfile,
        StartupLocation::Custom,
    ] {
        let mut prefs = saved_workspace_preferences();
        prefs.startup_location = location;
        prefs.startup_custom_path = temp.path().to_string_lossy().into_owned();
        let expected = location.resolve(&prefs.startup_custom_path);
        assert_eq!(prefs.prepare_fixed_startup(None), expected);
        assert!(prefs.saved_tab_session.is_none());
        assert!(!prefs.dual_panel_enabled);
        assert!(prefs.dual_panel_inactive_path.is_none());
        assert!(prefs.active_tag_filter.is_none());
    }
}

#[test]
fn previous_session_preserves_tabs_panels_and_tag_filter() {
    let mut prefs = saved_workspace_preferences();
    assert_eq!(prefs.prepare_fixed_startup(None), None);
    assert_eq!(
        prefs
            .saved_tab_session
            .unwrap()
            .into_tab_manager()
            .tabs
            .len(),
        2
    );
    assert!(prefs.dual_panel_enabled);
    assert!(prefs.dual_panel_inactive_path.is_some());
    assert_eq!(prefs.active_tag_filter, Some(1));
}

#[test]
fn shell_folder_takes_priority_in_fixed_location_mode() {
    let mut prefs = saved_workspace_preferences();
    prefs.startup_location = StartupLocation::Computer;
    assert_eq!(
        prefs.prepare_fixed_startup(Some(r"D:\Documents")),
        Some(r"D:\Documents".to_owned())
    );
    assert!(prefs.saved_tab_session.is_none());
    assert!(!prefs.dual_panel_enabled);
}

#[test]
fn startup_and_date_preferences_default_and_load_saved_values() {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    let defaults = StartupPreferences::load(&db);
    assert_eq!(defaults.startup_location, StartupLocation::PreviousSession);
    assert!(defaults.startup_custom_path.is_empty());
    assert_eq!(defaults.date_format, DateFormat::DayMonthYear);
    for location in [
        StartupLocation::Computer,
        StartupLocation::Documents,
        StartupLocation::UserProfile,
        StartupLocation::Custom,
    ] {
        db.set_preference("startup_location", location.preference_value())
            .unwrap();
        assert_eq!(StartupPreferences::load(&db).startup_location, location);
    }
    for format in [
        DateFormat::DayMonthYear,
        DateFormat::MonthDayYear,
        DateFormat::YearMonthDay,
    ] {
        db.set_preference("date_format", format.preference_value())
            .unwrap();
        assert_eq!(StartupPreferences::load(&db).date_format, format);
    }
    db.set_preference("startup_custom_path", r"D:\Documents\Project")
        .unwrap();
    assert_eq!(
        StartupPreferences::load(&db).startup_custom_path,
        r"D:\Documents\Project"
    );
    db.set_preference("startup_location", "invalid").unwrap();
    db.set_preference("date_format", "invalid").unwrap();
    let invalid = StartupPreferences::load(&db);
    assert_eq!(invalid.startup_location, StartupLocation::PreviousSession);
    assert_eq!(invalid.date_format, DateFormat::DayMonthYear);
}

#[test]
fn tab_preferences_load_independently_from_startup_location() {
    use super::tab_preferences::NewTabLocation;
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    let defaults = StartupPreferences::load(&db);
    assert_eq!(defaults.tab_preferences.location, NewTabLocation::Current);
    assert!(!defaults.tab_preferences.archives_in_new_tab);
    assert!(defaults.tab_preferences.activate_new_tabs);
    db.set_preference("startup_location", "documents").unwrap();
    db.set_preference("new_tab_location", "custom").unwrap();
    db.set_preference("new_tab_custom_path", r"D:\Documents\Project")
        .unwrap();
    db.set_preference("archives_in_new_tab", "true").unwrap();
    db.set_preference("activate_new_tabs", "false").unwrap();
    let loaded = StartupPreferences::load(&db);
    assert_eq!(loaded.startup_location, StartupLocation::Documents);
    assert_eq!(loaded.tab_preferences.location, NewTabLocation::Custom);
    assert_eq!(loaded.tab_preferences.custom_path, r"D:\Documents\Project");
    assert!(loaded.tab_preferences.archives_in_new_tab);
    assert!(!loaded.tab_preferences.activate_new_tabs);
    db.set_preference("new_tab_location", "invalid").unwrap();
    assert_eq!(
        StartupPreferences::load(&db).tab_preferences.location,
        NewTabLocation::Current
    );
}
