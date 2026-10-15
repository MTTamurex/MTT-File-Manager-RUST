use crate::app::init_preferences::StartupPreferences;
use crate::domain::special_paths::COMPUTER_VIEW_ID;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartupLocation {
    #[default]
    PreviousSession,
    Computer,
    Documents,
    UserProfile,
    Custom,
}

impl StartupLocation {
    pub fn from_preference(value: &str) -> Self {
        match value {
            "computer" => Self::Computer,
            "documents" => Self::Documents,
            "user_profile" => Self::UserProfile,
            "custom" => Self::Custom,
            _ => Self::PreviousSession,
        }
    }

    pub fn preference_value(self) -> &'static str {
        match self {
            Self::PreviousSession => "previous_session",
            Self::Computer => "computer",
            Self::Documents => "documents",
            Self::UserProfile => "user_profile",
            Self::Custom => "custom",
        }
    }

    pub fn resolve(self, custom_path: &str) -> Option<String> {
        let path = match self {
            Self::PreviousSession => return None,
            Self::Computer => Some(COMPUTER_VIEW_ID.to_owned()),
            Self::Documents => dirs::document_dir().map(|p| p.to_string_lossy().into_owned()),
            Self::UserProfile => dirs::home_dir().map(|p| p.to_string_lossy().into_owned()),
            Self::Custom => Some(custom_path.to_owned()),
        };
        Some(
            path.filter(|path| super::startup_folder::is_valid_startup_folder_path(path))
                .unwrap_or_else(|| COMPUTER_VIEW_ID.to_owned()),
        )
    }
}

impl StartupPreferences {
    /// Select a single initial location before constructing any restored tabs or panels.
    pub(super) fn prepare_fixed_startup(
        &mut self,
        requested_folder: Option<&str>,
    ) -> Option<String> {
        if self.startup_location == StartupLocation::PreviousSession {
            return None;
        }
        let path = requested_folder
            .map(str::to_owned)
            .or_else(|| self.startup_location.resolve(&self.startup_custom_path));
        self.saved_tab_session = None;
        self.active_tag_filter = None;
        self.dual_panel_enabled = false;
        self.dual_panel_inactive_path = None;
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_location_falls_back_without_changing_preference() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        assert_eq!(
            StartupLocation::Custom
                .resolve(&missing.to_string_lossy())
                .as_deref(),
            Some(COMPUTER_VIEW_ID)
        );
        assert_eq!(StartupLocation::PreviousSession.resolve(""), None);
        assert_eq!(
            StartupLocation::Computer.resolve("").as_deref(),
            Some(COMPUTER_VIEW_ID)
        );
        assert_eq!(
            StartupLocation::Custom
                .resolve(&temp.path().to_string_lossy())
                .as_deref(),
            Some(temp.path().to_string_lossy().as_ref())
        );
    }
}
