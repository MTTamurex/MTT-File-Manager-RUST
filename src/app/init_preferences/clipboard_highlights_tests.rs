use super::StartupPreferences;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::ui::clipboard_feedback::{HighlightPreferences, PREFERENCE_KEY};

#[test]
fn clipboard_highlights_load_defaults_and_saved_colors() {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    assert_eq!(
        StartupPreferences::load(&db).clipboard_highlights,
        HighlightPreferences::default()
    );
    let preferences = HighlightPreferences {
        enabled: false,
        copy_color: Some([10, 20, 30]),
        cut_color: Some([40, 50, 60]),
    };
    db.set_preference(PREFERENCE_KEY, &preferences.serialize())
        .unwrap();
    assert_eq!(
        StartupPreferences::load(&db).clipboard_highlights,
        preferences
    );
    db.set_preference(PREFERENCE_KEY, "invalid").unwrap();
    assert_eq!(
        StartupPreferences::load(&db).clipboard_highlights,
        HighlightPreferences::default()
    );
}
