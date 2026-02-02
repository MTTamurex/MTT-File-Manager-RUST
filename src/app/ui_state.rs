use eframe::egui;

/// Estado de UI
#[derive(Clone)]
pub struct UIState {
    pub ui_ctx: egui::Context,
    pub show_hidden_files: bool,
    pub show_thumbnails: bool,
    pub thumbnail_size: f32,
    pub grid_padding: f32,
    pub scroll_offset: egui::Vec2,
    pub selected_items: Vec<usize>,
    pub hovered_item: Option<usize>,
    pub rename_index: Option<usize>,
    pub rename_text: String,
    pub search_query: String,
    pub is_searching: bool,
    pub show_settings: bool,
    pub show_about: bool,
    pub show_help: bool,
    pub error_message: Option<String>,
    pub success_message: Option<String>,
    pub message_timeout: Option<std::time::Instant>,
    pub last_click_time: std::time::Instant,
    pub last_click_index: Option<usize>,
    pub drag_start_pos: Option<egui::Pos2>,
    pub is_dragging: bool,
    pub drag_threshold: f32,
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

impl UIState {
    pub fn new(ui_ctx: egui::Context) -> Self {
        Self {
            ui_ctx,
            show_hidden_files: false,
            show_thumbnails: true,
            thumbnail_size: 128.0,
            grid_padding: 8.0,
            scroll_offset: egui::Vec2::ZERO,
            selected_items: Vec::new(),
            hovered_item: None,
            rename_index: None,
            rename_text: String::new(),
            search_query: String::new(),
            is_searching: false,
            show_settings: false,
            show_about: false,
            show_help: false,
            error_message: None,
            success_message: None,
            message_timeout: None,
            last_click_time: std::time::Instant::now(),
            last_click_index: None,
            drag_start_pos: None,
            is_dragging: false,
            drag_threshold: 5.0,
            // Default column widths for list view - Regular
            list_col_name_width: 300.0,
            list_col_date_width: 170.0,
            list_col_type_width: 120.0,
            list_col_size_width: 100.0,
            // Default column widths for list view - OneDrive
            list_col_onedrive_name_width: 300.0,
            list_col_onedrive_date_width: 170.0,
            list_col_onedrive_type_width: 120.0,
            list_col_onedrive_size_width: 100.0,
            list_col_onedrive_status_width: 120.0,
            // Default column widths for list view - Computer
            list_col_computer_name_width: 300.0,
            list_col_computer_total_width: 120.0,
            list_col_computer_free_width: 120.0,
        }
    }
    
    /// Limpa estado de UI
    pub fn clear(&mut self) {
        self.selected_items.clear();
        self.hovered_item = None;
        self.rename_index = None;
        self.rename_text.clear();
        self.search_query.clear();
        self.is_searching = false;
        self.error_message = None;
        self.success_message = None;
        self.message_timeout = None;
        self.drag_start_pos = None;
        self.is_dragging = false;
    }
}