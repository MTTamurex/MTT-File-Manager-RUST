use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::ViewMode;
use std::time::Instant;

/// Navigation state
#[derive(Clone, Debug)]
pub struct NavigationState {
    pub current_path: String,
    pub history: NavigationHistory,
    pub path_input: String,
    pub disks: Vec<(String, String)>,
    pub last_drive_refresh: Instant,
    pub view_mode: ViewMode,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub computer_view_local_indices: Vec<usize>,
    pub computer_view_network_indices: Vec<usize>,
    pub show_virtual_drive_settings: bool,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self {
            current_path: String::new(),
            history: NavigationHistory::new(String::new()),
            path_input: String::new(),
            disks: Vec::new(),
            last_drive_refresh: Instant::now(),
            view_mode: ViewMode::Grid,
            is_computer_view: false,
            is_recycle_bin_view: false,
            computer_view_local_indices: Vec::new(),
            computer_view_network_indices: Vec::new(),
            show_virtual_drive_settings: false,
        }
    }
}
