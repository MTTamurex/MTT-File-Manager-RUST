use super::StartupPreferences;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::ui::list_stripes::PREFERENCE_KEY;

#[test]
fn list_stripes_default_off_and_load_saved_preference() {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    assert!(!StartupPreferences::load(&db).list_stripes_enabled);
    for (value, expected) in [
        ("true", true),
        ("false", false),
        ("invalid", false),
        ("", false),
    ] {
        db.set_preference(PREFERENCE_KEY, value).unwrap();
        assert_eq!(StartupPreferences::load(&db).list_stripes_enabled, expected);
    }
}
