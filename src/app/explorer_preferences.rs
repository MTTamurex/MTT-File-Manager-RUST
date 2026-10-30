use std::collections::HashMap;

/// How the detailed list decides column widths.
/// `AutoFit`: sizes columns to content, spare space goes to the name column.
/// `RememberPerFolder`: manual widths edited by the user, saved per folder.
/// `FitToPanel`: scales every visible column so they all fit in the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListColumnSizeMode {
    #[default]
    AutoFit,
    RememberPerFolder,
    FitToPanel,
}

impl ListColumnSizeMode {
    fn persisted_value(self) -> &'static str {
        match self {
            Self::AutoFit => "auto",
            Self::RememberPerFolder => "remember",
            Self::FitToPanel => "fit_panel",
        }
    }

    fn from_persisted_value(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::AutoFit),
            "remember" => Some(Self::RememberPerFolder),
            "fit_panel" => Some(Self::FitToPanel),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplorerPreferences {
    pub column_size_mode: ListColumnSizeMode,
    pub hover_preview: bool,
    pub sidebar_unified_scroll: bool,
}

impl Default for ExplorerPreferences {
    fn default() -> Self {
        Self {
            column_size_mode: ListColumnSizeMode::AutoFit,
            hover_preview: true,
            sidebar_unified_scroll: false,
        }
    }
}

impl ExplorerPreferences {
    pub(crate) fn load(prefs: &HashMap<String, String>) -> Self {
        Self {
            column_size_mode: prefs
                .get("list_column_size_mode")
                .and_then(|value| ListColumnSizeMode::from_persisted_value(value))
                .unwrap_or_else(|| {
                    // Databases saved before the three-mode picker kept a pair of
                    // booleans; "remember" was always on when automatic sizing was
                    // off, so that pair maps onto the two legacy modes.
                    let auto_resize_columns = prefs
                        .get("auto_resize_columns")
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(true);
                    if auto_resize_columns {
                        ListColumnSizeMode::AutoFit
                    } else {
                        ListColumnSizeMode::RememberPerFolder
                    }
                }),
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

    pub(crate) fn preferences(self) -> [(&'static str, String); 3] {
        [
            (
                "list_column_size_mode",
                self.column_size_mode.persisted_value().to_string(),
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
            ("list_column_size_mode".into(), "invalid".into()),
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
    fn legacy_boolean_preferences_map_to_the_matching_modes() {
        let auto = HashMap::from([
            ("auto_resize_columns".to_string(), "true".to_string()),
            ("remember_column_widths".to_string(), "false".to_string()),
        ]);
        assert_eq!(
            ExplorerPreferences::load(&auto).column_size_mode,
            ListColumnSizeMode::AutoFit
        );
        let remember = HashMap::from([
            ("auto_resize_columns".to_string(), "false".to_string()),
            ("remember_column_widths".to_string(), "true".to_string()),
        ]);
        assert_eq!(
            ExplorerPreferences::load(&remember).column_size_mode,
            ListColumnSizeMode::RememberPerFolder
        );
        // The three-mode key wins whenever it is present and valid.
        let both = HashMap::from([
            ("list_column_size_mode".to_string(), "fit_panel".to_string()),
            ("auto_resize_columns".to_string(), "false".to_string()),
        ]);
        assert_eq!(
            ExplorerPreferences::load(&both).column_size_mode,
            ListColumnSizeMode::FitToPanel
        );
    }

    #[test]
    fn options_round_trip_independently() {
        for column_size_mode in [
            ListColumnSizeMode::AutoFit,
            ListColumnSizeMode::RememberPerFolder,
            ListColumnSizeMode::FitToPanel,
        ] {
            for hover_preview in [false, true] {
                for sidebar_unified_scroll in [false, true] {
                    let options = ExplorerPreferences {
                        column_size_mode,
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
            column_size_mode: ListColumnSizeMode::RememberPerFolder,
            hover_preview: false,
            sidebar_unified_scroll: true,
        };
        for (key, value) in options.preferences() {
            db.set_preference(key, &value).unwrap();
        }
        assert_eq!(StartupPreferences::load(&db).explorer_preferences, options);
    }

    #[test]
    fn startup_migrates_legacy_boolean_mode_from_the_database() {
        use crate::app::init_preferences::StartupPreferences;
        use crate::infrastructure::app_state_db::AppStateDb;

        let db = AppStateDb::new_in_memory().unwrap();
        db.set_preference("auto_resize_columns", "false").unwrap();
        db.set_preference("remember_column_widths", "true").unwrap();
        assert_eq!(
            StartupPreferences::load(&db)
                .explorer_preferences
                .column_size_mode,
            ListColumnSizeMode::RememberPerFolder
        );
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
