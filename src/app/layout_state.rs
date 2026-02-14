/// State grouping for window/layout-related preferences and runtime sizes.
#[derive(Debug, Clone)]
pub struct LayoutState {
    // Window state persistence
    pub saved_window_width: f32,
    pub saved_window_height: f32,
    pub saved_is_maximized: bool,
    pub saved_is_minimized: bool,

    // Sidebar widths persistence
    pub sidebar_left_width: f32,
    pub sidebar_right_width: f32,

    // List view column widths (resizable) - Regular view
    pub list_col_name_width: f32,
    pub list_col_date_width: f32,
    pub list_col_type_width: f32,
    pub list_col_size_width: f32,

    // List view column widths - OneDrive view
    pub list_col_onedrive_name_width: f32,
    pub list_col_onedrive_date_width: f32,
    pub list_col_onedrive_type_width: f32,
    pub list_col_onedrive_size_width: f32,
    pub list_col_onedrive_status_width: f32,

    // List view column widths - Computer view
    pub list_col_computer_name_width: f32,
    pub list_col_computer_total_width: f32,
    pub list_col_computer_free_width: f32,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            saved_window_width: 1280.0,
            saved_window_height: 720.0,
            saved_is_maximized: true,
            saved_is_minimized: false,
            sidebar_left_width: 200.0,
            sidebar_right_width: 300.0,
            list_col_name_width: 300.0,
            list_col_date_width: 170.0,
            list_col_type_width: 120.0,
            list_col_size_width: 100.0,
            list_col_onedrive_name_width: 300.0,
            list_col_onedrive_date_width: 170.0,
            list_col_onedrive_type_width: 120.0,
            list_col_onedrive_size_width: 100.0,
            list_col_onedrive_status_width: 120.0,
            list_col_computer_name_width: 300.0,
            list_col_computer_total_width: 120.0,
            list_col_computer_free_width: 120.0,
        }
    }
}
