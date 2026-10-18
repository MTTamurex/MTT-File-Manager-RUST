use crate::application::navigation::NavigationHistory;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuickAccessPlacement {
    #[default]
    Sidebar,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
    DarkDeep,
    GitHubDark,
    System,
}

impl ThemeMode {
    pub fn from_preference(value: &str) -> Self {
        match value {
            "dark" => Self::Dark,
            "dark_deep" => Self::DarkDeep,
            "github_dark" => Self::GitHubDark,
            "system" => Self::System,
            _ => Self::Light,
        }
    }

    pub fn preference(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::DarkDeep => "dark_deep",
            Self::GitHubDark => "github_dark",
            Self::System => "system",
        }
    }

    /// Whether this mode should render dark right now. `System` follows the
    /// Windows "app mode" setting, falling back to light when it is unavailable.
    pub fn resolve_dark(self) -> bool {
        match self {
            ThemeMode::Light => false,
            ThemeMode::Dark | ThemeMode::DarkDeep | ThemeMode::GitHubDark => true,
            ThemeMode::System => crate::infrastructure::windows::windows_app_is_dark(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Appearance,
    Diagnostics,
    Graphics,
    Shortcuts,
    Tags,
    Organizer,
    VirtualDrives,
    About,
}

/// Navigation state
#[derive(Clone, Debug)]
pub struct NavigationState {
    pub current_path: String,
    pub navigation: NavigationHistory,
    pub path_input: String,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub computer_view_local_indices: Vec<usize>,
    pub computer_view_network_indices: Vec<usize>,
    pub show_settings_window: bool,
    pub active_settings_section: SettingsSection,
}

impl NavigationState {
    pub fn new(current_path: String, is_computer_view: bool) -> Self {
        Self {
            navigation: NavigationHistory::new(current_path.clone()),
            path_input: current_path.clone(),
            current_path,
            is_computer_view,
            is_recycle_bin_view: false,
            computer_view_local_indices: Vec::new(),
            computer_view_network_indices: Vec::new(),
            show_settings_window: false,
            active_settings_section: SettingsSection::General,
        }
    }
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::new(String::new(), false)
    }
}
