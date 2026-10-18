use super::StartupPreferences;
use crate::domain::folder_appearance::FolderAppearance;
use crate::infrastructure::app_state_db::AppStateDb;

#[test]
fn folder_appearance_loads_saved_choice_independently_of_theme() {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference("language", "en").unwrap();
    assert_eq!(
        StartupPreferences::load(&db).folder_appearance,
        FolderAppearance::Original
    );

    for theme in ["light", "dark", "dark_deep", "github_dark", "system"] {
        db.set_preference("theme_mode", theme).unwrap();
        for appearance in FolderAppearance::ALL {
            db.set_preference(FolderAppearance::PREFERENCE_KEY, appearance.preference())
                .unwrap();
            assert_eq!(StartupPreferences::load(&db).folder_appearance, appearance);
        }
    }

    db.set_preference(FolderAppearance::PREFERENCE_KEY, "soft")
        .unwrap();
    assert_eq!(
        StartupPreferences::load(&db).folder_appearance,
        FolderAppearance::PastelYellow
    );

    db.set_preference(FolderAppearance::PREFERENCE_KEY, "invalid")
        .unwrap();
    assert_eq!(
        StartupPreferences::load(&db).folder_appearance,
        FolderAppearance::Original
    );
}
