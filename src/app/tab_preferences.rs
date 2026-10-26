use super::startup_location::StartupLocation;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewTabLocation {
    #[default]
    Current,
    Computer,
    Documents,
    UserProfile,
    Custom,
}

impl NewTabLocation {
    pub fn from_preference(value: &str) -> Self {
        match value {
            "computer" => Self::Computer,
            "documents" => Self::Documents,
            "user_profile" => Self::UserProfile,
            "custom" => Self::Custom,
            _ => Self::Current,
        }
    }

    pub fn preference_value(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Computer => "computer",
            Self::Documents => "documents",
            Self::UserProfile => "user_profile",
            Self::Custom => "custom",
        }
    }

    pub fn resolve(self, current: &str, custom: &str) -> String {
        let location = match self {
            Self::Current => return current.to_owned(),
            Self::Computer => StartupLocation::Computer,
            Self::Documents => StartupLocation::Documents,
            Self::UserProfile => StartupLocation::UserProfile,
            Self::Custom => StartupLocation::Custom,
        };
        location.resolve(custom).expect("fixed locations resolve")
    }
}

#[derive(Clone, Debug)]
pub struct TabPreferences {
    pub location: NewTabLocation,
    pub custom_path: String,
    pub archives_in_new_tab: bool,
    pub activate_new_tabs: bool,
}

impl Default for TabPreferences {
    fn default() -> Self {
        Self {
            location: NewTabLocation::default(),
            custom_path: String::new(),
            archives_in_new_tab: false,
            activate_new_tabs: true,
        }
    }
}

impl TabPreferences {
    pub fn should_activate_new_tab(&self, shift: bool) -> bool {
        self.activate_new_tabs != shift
    }

    /// A candidate only: physical file classification happens off the UI thread.
    pub fn archive_activation_candidate(&self, path: &str) -> bool {
        self.archives_in_new_tab && crate::domain::file_entry::is_archive_extension(path)
    }

    pub fn load(prefs: &HashMap<String, String>) -> Self {
        Self {
            location: NewTabLocation::from_preference(
                prefs
                    .get("new_tab_location")
                    .map(String::as_str)
                    .unwrap_or_default(),
            ),
            custom_path: prefs
                .get("new_tab_custom_path")
                .cloned()
                .unwrap_or_default(),
            archives_in_new_tab: prefs
                .get("archives_in_new_tab")
                .is_some_and(|v| v == "true"),
            activate_new_tabs: prefs.get("activate_new_tabs").is_none_or(|v| v != "false"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};

    #[test]
    fn shift_reverses_focus_preference_and_existing_profiles_default_to_activation() {
        assert!(TabPreferences::load(&HashMap::new()).activate_new_tabs);
        for enabled in [false, true] {
            let prefs = TabPreferences {
                activate_new_tabs: enabled,
                ..Default::default()
            };
            assert_eq!(prefs.should_activate_new_tab(false), enabled);
            assert_eq!(prefs.should_activate_new_tab(true), !enabled);
        }
    }

    #[test]
    fn destinations_preserve_virtual_locations_and_fall_back_for_missing_folders() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        assert_eq!(
            NewTabLocation::Current.resolve(RECYCLE_BIN_VIEW_ID, ""),
            RECYCLE_BIN_VIEW_ID
        );
        assert_eq!(NewTabLocation::Computer.resolve("", ""), COMPUTER_VIEW_ID);
        assert_eq!(
            NewTabLocation::Custom.resolve("", &missing.to_string_lossy()),
            COMPUTER_VIEW_ID
        );
        assert_eq!(
            NewTabLocation::Custom.resolve("", &temp.path().to_string_lossy()),
            temp.path().to_string_lossy()
        );
    }

    #[test]
    fn archive_candidates_do_not_require_filesystem_access() {
        let mut prefs = TabPreferences::default();
        assert!(!prefs.archive_activation_candidate(r"\\server\share\sample.ZIP"));
        prefs.archives_in_new_tab = true;
        assert!(prefs.archive_activation_candidate(r"\\server\share\sample.ZIP"));
        assert!(!prefs.archive_activation_candidate(r"\\server\share\sample.zip\inner"));
        assert!(!prefs.archive_activation_candidate("file.txt"));
    }
}
