use crate::application::navigation::NavigationHistory;

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
    pub show_virtual_drive_settings: bool,
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
            show_virtual_drive_settings: false,
        }
    }
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::new(String::new(), false)
    }
}
