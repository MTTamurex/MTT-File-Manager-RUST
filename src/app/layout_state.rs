/// State grouping for window/layout-related preferences and runtime sizes.
#[derive(Debug, Clone)]
pub struct LayoutState {
    // Window state persistence
    pub saved_window_width: f32,
    pub saved_window_height: f32,
    pub saved_is_maximized: bool,
    pub saved_is_minimized: bool,
    pub saved_is_fullscreen: bool,

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

    // Dual panel split ratio (default 0.5 for equal split)
    pub dual_panel_split_ratio: f32,

    // Quick Access section height (None = content-sized, Some = user-resized)
    pub sidebar_quick_access_height: Option<f32>,
    // Drives viewport height (None = automatic content-sized layout).
    pub sidebar_drives_height: Option<f32>,
    // Runtime limits for each combination of collapsed cloud/local/network sections.
    pub sidebar_drives_heights: std::collections::HashMap<[bool; 4], Option<f32>>,
    // Preserve the expanded divider while Collapse All temporarily fits headers.
    pub sidebar_drives_height_before_collapse_all: Option<Option<f32>>,
}

impl Default for LayoutState {
    fn default() -> Self {
        Self {
            saved_window_width: 1280.0,
            saved_window_height: 720.0,
            saved_is_maximized: true,
            saved_is_minimized: false,
            saved_is_fullscreen: false,
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
            dual_panel_split_ratio: 0.5,
            sidebar_quick_access_height: None,
            sidebar_drives_height: None,
            sidebar_drives_heights: Default::default(),
            sidebar_drives_height_before_collapse_all: None,
        }
    }
}

impl LayoutState {
    pub(crate) fn set_sidebar_sections_collapsed(&mut self, collapsed: bool) {
        if collapsed {
            self.sidebar_drives_height_before_collapse_all
                .get_or_insert(self.sidebar_drives_height);
            self.sidebar_drives_height = None;
        } else if let Some(saved) = self.sidebar_drives_height_before_collapse_all.take() {
            self.sidebar_drives_height = saved;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LayoutState;

    #[test]
    fn expand_all_restores_dividers_without_changing_other_layout_preferences() {
        for saved in [None, Some(180.0)] {
            let mut layout = LayoutState {
                sidebar_quick_access_height: Some(120.0),
                sidebar_drives_height: saved,
                sidebar_drives_heights: [([false; 4], saved), ([true; 4], Some(60.0))].into(),
                ..Default::default()
            };
            let remembered = layout.sidebar_drives_heights.clone();
            let width = layout.sidebar_left_width;
            layout.set_sidebar_sections_collapsed(true);
            assert_eq!(layout.sidebar_drives_height, None);
            // Resizing the collapsed layout must not replace the expanded size.
            layout.sidebar_drives_height = Some(80.0);
            layout.set_sidebar_sections_collapsed(true);
            assert_eq!(
                layout.sidebar_drives_height_before_collapse_all,
                Some(saved)
            );
            layout.set_sidebar_sections_collapsed(false);
            assert_eq!(layout.sidebar_drives_height, saved);
            assert_eq!(layout.sidebar_drives_height_before_collapse_all, None);
            assert_eq!(layout.sidebar_quick_access_height, Some(120.0));
            assert_eq!(layout.sidebar_drives_heights, remembered);
            assert_eq!(layout.sidebar_left_width, width);

            // The next cycle remembers newly adjusted expanded dividers.
            layout.sidebar_drives_height = Some(220.0);
            layout.set_sidebar_sections_collapsed(true);
            layout.set_sidebar_sections_collapsed(false);
            assert_eq!(layout.sidebar_drives_height, Some(220.0));
        }
    }

    #[test]
    fn expand_all_without_a_previous_collapse_keeps_current_heights() {
        let mut layout = LayoutState {
            sidebar_drives_height: Some(180.0),
            sidebar_quick_access_height: Some(120.0),
            ..Default::default()
        };
        layout.set_sidebar_sections_collapsed(false);
        assert_eq!(layout.sidebar_drives_height, Some(180.0));
        assert_eq!(layout.sidebar_quick_access_height, Some(120.0));
    }
}
