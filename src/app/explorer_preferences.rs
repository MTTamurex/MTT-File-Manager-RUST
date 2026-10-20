use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplorerPreferences {
    pub auto_resize_columns: bool,
    pub hover_preview: bool,
}

impl Default for ExplorerPreferences {
    fn default() -> Self {
        Self {
            auto_resize_columns: true,
            hover_preview: true,
        }
    }
}

impl ExplorerPreferences {
    pub(crate) fn load(prefs: &HashMap<String, String>) -> Self {
        Self {
            auto_resize_columns: prefs
                .get("auto_resize_columns")
                .and_then(|value| value.parse().ok())
                .unwrap_or(true),
            hover_preview: prefs
                .get("hover_preview")
                .and_then(|value| value.parse().ok())
                .unwrap_or(true),
        }
    }

    pub(crate) fn preferences(self) -> [(&'static str, String); 2] {
        [
            ("auto_resize_columns", self.auto_resize_columns.to_string()),
            ("hover_preview", self.hover_preview.to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_invalid_preferences_preserve_current_behavior() {
        assert_eq!(
            ExplorerPreferences::load(&HashMap::new()),
            ExplorerPreferences::default()
        );
        let prefs = HashMap::from([
            ("auto_resize_columns".into(), "invalid".into()),
            ("hover_preview".into(), "invalid".into()),
        ]);
        assert_eq!(
            ExplorerPreferences::load(&prefs),
            ExplorerPreferences::default()
        );
    }

    #[test]
    fn options_round_trip_independently() {
        for auto_resize_columns in [false, true] {
            for hover_preview in [false, true] {
                let options = ExplorerPreferences {
                    auto_resize_columns,
                    hover_preview,
                };
                let prefs = options
                    .preferences()
                    .into_iter()
                    .map(|(key, value)| (key.to_string(), value))
                    .collect();
                assert_eq!(ExplorerPreferences::load(&prefs), options);
            }
        }
    }

    #[test]
    fn startup_loads_saved_options_from_database() {
        use crate::app::init_preferences::StartupPreferences;
        use crate::infrastructure::app_state_db::AppStateDb;

        let db = AppStateDb::new_in_memory().unwrap();
        assert_eq!(
            StartupPreferences::load(&db).explorer_preferences,
            ExplorerPreferences::default()
        );
        let options = ExplorerPreferences {
            auto_resize_columns: false,
            hover_preview: false,
        };
        for (key, value) in options.preferences() {
            db.set_preference(key, &value).unwrap();
        }
        assert_eq!(StartupPreferences::load(&db).explorer_preferences, options);
    }
}
