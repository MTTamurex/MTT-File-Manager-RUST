use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplorerPreferences {
    pub auto_resize_columns: bool,
    pub remember_column_widths: bool,
    pub hover_preview: bool,
    pub sidebar_unified_scroll: bool,
}

impl Default for ExplorerPreferences {
    fn default() -> Self {
        Self {
            auto_resize_columns: true,
            remember_column_widths: false,
            hover_preview: true,
            sidebar_unified_scroll: false,
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
            remember_column_widths: prefs
                .get("remember_column_widths")
                .and_then(|value| value.parse().ok())
                .unwrap_or(false),
            hover_preview: prefs
                .get("hover_preview")
                .and_then(|value| value.parse().ok())
                .unwrap_or(true),
            sidebar_unified_scroll: prefs
                .get("sidebar_unified_scroll")
                .and_then(|value| value.parse().ok())
                .unwrap_or(false),
        }
    }

    pub(crate) fn preferences(self) -> [(&'static str, String); 4] {
        [
            ("auto_resize_columns", self.auto_resize_columns.to_string()),
            (
                "remember_column_widths",
                self.remember_column_widths.to_string(),
            ),
            ("hover_preview", self.hover_preview.to_string()),
            (
                "sidebar_unified_scroll",
                self.sidebar_unified_scroll.to_string(),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_invalid_preferences_preserve_current_behavior() {
        assert!(!ExplorerPreferences::default().sidebar_unified_scroll);
        assert_eq!(
            ExplorerPreferences::load(&HashMap::new()),
            ExplorerPreferences::default()
        );
        let prefs = HashMap::from([
            ("auto_resize_columns".into(), "invalid".into()),
            ("remember_column_widths".into(), "invalid".into()),
            ("hover_preview".into(), "invalid".into()),
            ("sidebar_unified_scroll".into(), "invalid".into()),
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
                for remember_column_widths in [false, true] {
                    for sidebar_unified_scroll in [false, true] {
                        let options = ExplorerPreferences {
                            auto_resize_columns,
                            remember_column_widths,
                            hover_preview,
                            sidebar_unified_scroll,
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
            remember_column_widths: true,
            hover_preview: false,
            sidebar_unified_scroll: true,
        };
        for (key, value) in options.preferences() {
            db.set_preference(key, &value).unwrap();
        }
        assert_eq!(StartupPreferences::load(&db).explorer_preferences, options);
    }

    #[test]
    fn sidebar_scroll_toggle_preserves_saved_section_heights() {
        use crate::app::init_preferences::StartupPreferences;
        use crate::infrastructure::app_state_db::AppStateDb;

        let db = AppStateDb::new_in_memory().unwrap();
        db.set_preference("sidebar_quick_access_height", "140")
            .unwrap();
        db.set_preference("sidebar_drives_height", "280").unwrap();
        for enabled in [true, false, true, false] {
            let options = ExplorerPreferences {
                sidebar_unified_scroll: enabled,
                ..Default::default()
            };
            for (key, value) in options.preferences() {
                db.set_preference(key, &value).unwrap();
            }
            let loaded = StartupPreferences::load(&db);
            assert_eq!(loaded.explorer_preferences.sidebar_unified_scroll, enabled);
            assert_eq!(loaded.sidebar_quick_access_height, Some(140.0));
            assert_eq!(loaded.sidebar_drives_height, Some(280.0));
        }
    }
}
