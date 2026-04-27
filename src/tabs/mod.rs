//! Tab management system for MTT File Manager
//!
//! Each tab represents an independent file browser view with its own:
//! - Current path
//! - Navigation history
//! - Selected items
//! - Sort preferences

use crate::app::dual_panel::{ActivePanel, PanelSnapshot};
use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use rustc_hash::FxHashSet;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn tab_title_for_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Represents a single browser tab
#[derive(Clone)]
pub struct TabState {
    /// Unique identifier for this tab
    pub id: usize,
    /// Current directory path
    pub path: String,
    /// Display title (folder name or "Este Computador")
    pub title: String,
    /// Navigation history (linear)
    pub navigation: NavigationHistory,
    /// Whether this tab is showing "Este Computador" view
    pub is_computer_view: bool,
    /// Items in this tab's view
    pub items: Arc<Vec<FileEntry>>,
    /// Unfiltered items (for search)
    pub all_items: Arc<Vec<FileEntry>>,
    /// Whether `items` was intentionally compacted away because the visible
    /// snapshot is identical to `all_items` for this stored tab state.
    pub items_snapshot_compact: bool,
    /// Selected item index
    pub selected_item: Option<usize>,
    /// Selected file entry
    pub selected_file: Option<FileEntry>,
    /// Search query for this tab
    pub search_query: String,
    /// Whether to scroll to selected item on next frame
    pub scroll_to_selected: bool,
    /// Address bar input text
    pub path_input: String,
    /// Whether this tab is showing the Recycle Bin view
    pub is_recycle_bin_view: bool,
    /// Persistent thumbnail for preview panel
    pub selected_thumbnail: Option<eframe::egui::TextureHandle>,
    /// Selected metadata for preview panel
    pub selected_metadata: Option<(PathBuf, crate::infrastructure::windows::MediaMetadata)>,
    /// Selected animated GIF for local preview (native)
    pub selected_gif: Option<crate::ui::components::media_preview::GifPlayer>,
    /// Scroll offset for grid view (manual virtualization)
    pub scroll_offset_y: f32,
    /// Total items in the folder (status bar)
    pub total_items: usize,
    /// View mode for this tab (Grid or List)
    pub view_mode: ViewMode,
    /// Multi-selection set for this tab
    pub multi_selection: FxHashSet<PathBuf>,
    /// Sort mode for this tab
    pub sort_mode: SortMode,
    /// Sort direction for this tab
    pub sort_descending: bool,
    /// Folders position for this tab
    pub folders_position: FoldersPosition,
    /// Whether the left sidebar is visible in this tab
    pub show_left_sidebar: bool,
    /// Whether the right preview/details panel is visible in this tab
    pub show_preview_panel: bool,
    /// Quick search buffer (type-to-search like Explorer)
    pub quick_search_buffer: String,
    /// Last keystroke time for quick search timeout
    pub quick_search_last_input: std::time::Instant,
    /// Sidebar tree: which folders are expanded (per-tab)
    pub sidebar_expanded: HashSet<PathBuf>,
    /// Sidebar tree: scroll position (per-tab)
    pub sidebar_scroll_y: f32,
    /// Sidebar section collapse states (per-tab)
    pub collapse_quick_access: bool,
    pub collapse_local_disks: bool,
    pub collapse_network_drives: bool,
    // Dual panel state (per-tab)
    pub dual_panel_enabled: bool,
    pub dual_panel_active: ActivePanel,
    pub dual_panel_inactive_state: Option<PanelSnapshot>,
}

impl TabState {
    /// Create a new tab at "Este Computador"
    pub fn new_at_computer(id: usize) -> Self {
        Self {
            id,
            path: COMPUTER_VIEW_ID.to_string(),
            title: COMPUTER_VIEW_ID.to_string(),
            navigation: NavigationHistory::new(COMPUTER_VIEW_ID.to_string()),
            is_computer_view: true,
            items: Arc::new(Vec::new()),
            all_items: Arc::new(Vec::new()),
            items_snapshot_compact: false,
            selected_item: None,
            selected_file: None,
            search_query: String::new(),
            scroll_to_selected: false,
            path_input: COMPUTER_VIEW_ID.to_string(),
            is_recycle_bin_view: false,
            selected_thumbnail: None,
            selected_metadata: None,
            selected_gif: None,
            scroll_offset_y: 0.0,
            total_items: 0,
            view_mode: ViewMode::Grid,
            multi_selection: FxHashSet::default(),
            sort_mode: SortMode::Name,
            sort_descending: false,
            folders_position: FoldersPosition::First,
            show_left_sidebar: true,
            show_preview_panel: true,
            quick_search_buffer: String::new(),
            quick_search_last_input: std::time::Instant::now(),
            sidebar_expanded: HashSet::new(),
            sidebar_scroll_y: 0.0,
            collapse_quick_access: false,
            collapse_local_disks: false,
            collapse_network_drives: false,
            dual_panel_enabled: false,
            dual_panel_active: ActivePanel::Left,
            dual_panel_inactive_state: None,
        }
    }

    /// Create a new tab at a specific path
    pub fn new_at_path(id: usize, path: &str) -> Self {
        let title = tab_title_for_path(path);

        Self {
            id,
            path: path.to_string(),
            title,
            navigation: NavigationHistory::new(path.to_string()),
            is_computer_view: false,
            items: Arc::new(Vec::new()),
            all_items: Arc::new(Vec::new()),
            items_snapshot_compact: false,
            selected_item: None,
            selected_file: None,
            search_query: String::new(),
            scroll_to_selected: false,
            path_input: path.to_string(),
            is_recycle_bin_view: path == RECYCLE_BIN_VIEW_ID,
            selected_thumbnail: None,
            selected_metadata: None,
            selected_gif: None,
            scroll_offset_y: 0.0,
            total_items: 0,
            view_mode: ViewMode::Grid,
            multi_selection: FxHashSet::default(),
            sort_mode: SortMode::Name,
            sort_descending: false,
            folders_position: FoldersPosition::First,
            show_left_sidebar: true,
            show_preview_panel: true,
            quick_search_buffer: String::new(),
            quick_search_last_input: std::time::Instant::now(),
            sidebar_expanded: HashSet::new(),
            sidebar_scroll_y: 0.0,
            collapse_quick_access: false,
            collapse_local_disks: false,
            collapse_network_drives: false,
            dual_panel_enabled: false,
            dual_panel_active: ActivePanel::Left,
            dual_panel_inactive_state: None,
        }
    }

    /// Navigate to a new path, pushing current to history
    pub fn navigate_to(&mut self, new_path: &str) {
        if new_path == self.path {
            return;
        }

        // Delegate to navigation manager
        self.navigation.navigate_to(new_path.to_string());

        // Update current path
        self.path = new_path.to_string();
        self.path_input = new_path.to_string();
        self.is_computer_view = new_path == COMPUTER_VIEW_ID;
        self.is_recycle_bin_view = new_path == RECYCLE_BIN_VIEW_ID;
        self.scroll_offset_y = 0.0;

        // Update title
        if self.is_computer_view {
            self.title = COMPUTER_VIEW_ID.to_string();
        } else {
            self.title = tab_title_for_path(new_path);
        }
    }

    /// Go back in history
    pub fn go_back(&mut self) -> bool {
        if let Some(path) = self.navigation.go_back().cloned() {
            self.path = path.clone();
            self.sync_from_history();
            true
        } else {
            false
        }
    }

    /// Go forward in history
    pub fn go_forward(&mut self) -> bool {
        if let Some(path) = self.navigation.go_forward().cloned() {
            self.path = path.clone();
            self.sync_from_history();
            true
        } else {
            false
        }
    }

    fn sync_from_history(&mut self) {
        if let Some(path) = self.navigation.current_path() {
            self.path_input = path.clone();
            self.is_computer_view = path == COMPUTER_VIEW_ID;
            self.is_recycle_bin_view = path == RECYCLE_BIN_VIEW_ID;
            self.scroll_offset_y = 0.0;

            if self.is_computer_view {
                self.title = COMPUTER_VIEW_ID.to_string();
            } else {
                self.title = tab_title_for_path(path);
            }
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.navigation.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.navigation.can_go_forward()
    }

    pub fn restore_items_snapshot(&self) -> Arc<Vec<FileEntry>> {
        if self.items_snapshot_compact {
            self.all_items.clone()
        } else {
            self.items.clone()
        }
    }

    pub fn visible_items_len(&self) -> usize {
        if self.items_snapshot_compact {
            self.all_items.len()
        } else {
            self.items.len()
        }
    }

    /// Keep only lightweight state for closed-tab history to avoid retaining heavy caches.
    fn into_lightweight_closed_snapshot(mut self) -> Self {
        self.items = Arc::new(Vec::new());
        self.all_items = Arc::new(Vec::new());
        self.items_snapshot_compact = false;
        self.selected_item = None;
        self.selected_file = None;
        self.selected_thumbnail = None;
        self.selected_metadata = None;
        self.selected_gif = None;
        self.multi_selection.clear();
        self.scroll_offset_y = 0.0;
        self.total_items = 0;
        self.quick_search_buffer.clear();
        self.sidebar_expanded.clear();
        self.sidebar_scroll_y = 0.0;
        self.collapse_quick_access = false;
        self.collapse_local_disks = false;
        self.collapse_network_drives = false;
        self.dual_panel_enabled = false;
        self.dual_panel_inactive_state = None;
        self
    }
}

/// Manages all open tabs
pub struct TabManager {
    /// All open tabs
    pub tabs: Vec<TabState>,
    /// Index of the currently active tab
    pub active_tab: usize,
    /// Counter for generating unique tab IDs
    next_id: usize,
    /// Recently closed tabs (for reopening)
    closed_tabs: Vec<TabState>,
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TabManager {
    /// Create a new TabManager with one tab at "Este Computador"
    pub fn new() -> Self {
        let initial_tab = TabState::new_at_computer(0);
        Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            next_id: 1,
            closed_tabs: Vec::new(),
        }
    }

    /// Create a new TabManager with one tab at the specified path
    pub fn new_at_path(path: &str) -> Self {
        let initial_tab = TabState::new_at_path(0, path);
        Self {
            tabs: vec![initial_tab],
            active_tab: 0,
            next_id: 1,
            closed_tabs: Vec::new(),
        }
    }

    /// Get the currently active tab
    pub fn active(&self) -> &TabState {
        &self.tabs[self.active_tab]
    }

    /// Get mutable reference to active tab
    pub fn active_mut(&mut self) -> &mut TabState {
        &mut self.tabs[self.active_tab]
    }

    /// Add a new tab at "Este Computador" and switch to it
    pub fn new_tab(&mut self) {
        let mut tab = TabState::new_at_computer(self.next_id);
        let current = self.active();
        tab.show_left_sidebar = current.show_left_sidebar;
        tab.show_preview_panel = current.show_preview_panel;
        self.next_id += 1;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    /// Add a new tab at a specific path and switch to it
    pub fn new_tab_at(&mut self, path: &str) {
        let mut tab = TabState::new_at_path(self.next_id, path);
        let current = self.active();
        tab.show_left_sidebar = current.show_left_sidebar;
        tab.show_preview_panel = current.show_preview_panel;
        self.next_id += 1;
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
    }

    /// Duplicate the current tab
    pub fn duplicate_tab(&mut self) {
        let current = self.active().clone();
        let mut new_tab = TabState::new_at_path(self.next_id, &current.path);
        new_tab.navigation = current.navigation.clone();
        new_tab.is_computer_view = current.is_computer_view;
        new_tab.items = if current.items_snapshot_compact {
            Arc::new(Vec::new())
        } else {
            current.items.clone()
        };
        new_tab.all_items = current.all_items.clone();
        new_tab.items_snapshot_compact = current.items_snapshot_compact;
        new_tab.selected_item = current.selected_item;
        new_tab.selected_file = current.selected_file.clone();
        new_tab.selected_thumbnail = None;
        new_tab.selected_metadata = current.selected_metadata.clone();
        new_tab.selected_gif = None;
        new_tab.search_query = current.search_query.clone();
        new_tab.total_items = current.total_items;
        new_tab.view_mode = current.view_mode;
        new_tab.multi_selection = current.multi_selection.clone();
        new_tab.sort_mode = current.sort_mode;
        new_tab.sort_descending = current.sort_descending;
        new_tab.folders_position = current.folders_position;
        new_tab.show_left_sidebar = current.show_left_sidebar;
        new_tab.show_preview_panel = current.show_preview_panel;
        new_tab.sidebar_expanded = current.sidebar_expanded.clone();
        new_tab.sidebar_scroll_y = current.sidebar_scroll_y;

        self.next_id += 1;

        // Insert after current tab
        let insert_pos = self.active_tab + 1;
        self.tabs.insert(insert_pos, new_tab);
        self.active_tab = insert_pos;
    }

    /// Close the tab at the given index
    /// Returns true if the app should close (no tabs left)
    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 {
            // Last tab - signal app should close
            return true;
        }

        // Save to closed tabs for potential reopening
        let closed = self.tabs.remove(index).into_lightweight_closed_snapshot();
        self.closed_tabs.push(closed);

        // Keep max 10 closed tabs
        if self.closed_tabs.len() > 10 {
            self.closed_tabs.remove(0);
        }

        // Adjust active tab index
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > index {
            self.active_tab = self.active_tab.saturating_sub(1);
        }

        false
    }

    /// Close the currently active tab
    pub fn close_active_tab(&mut self) -> bool {
        self.close_tab(self.active_tab)
    }

    /// Switch to the tab at the given index
    pub fn switch_to(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = index;
        }
    }

    /// Switch to the next tab (wrapping around)
    pub fn next_tab(&mut self) {
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
    }

    /// Switch to the previous tab (wrapping around)
    pub fn prev_tab(&mut self) {
        if self.active_tab == 0 {
            self.active_tab = self.tabs.len() - 1;
        } else {
            self.active_tab -= 1;
        }
    }

    /// Reopen the most recently closed tab
    pub fn reopen_closed_tab(&mut self) -> bool {
        if let Some(tab) = self.closed_tabs.pop() {
            let mut reopened = TabState::new_at_path(self.next_id, &tab.path);
            reopened.navigation = tab.navigation;
            reopened.is_computer_view = tab.is_computer_view;
            reopened.items = tab.items;
            reopened.all_items = tab.all_items;
            reopened.items_snapshot_compact = tab.items_snapshot_compact;
            reopened.selected_item = tab.selected_item;
            reopened.selected_file = tab.selected_file;
            reopened.selected_thumbnail = tab.selected_thumbnail;
            reopened.selected_metadata = tab.selected_metadata;
            reopened.selected_gif = tab.selected_gif;
            reopened.search_query = tab.search_query;
            reopened.total_items = tab.total_items;
            reopened.view_mode = tab.view_mode;
            reopened.multi_selection = tab.multi_selection;
            reopened.sort_mode = tab.sort_mode;
            reopened.sort_descending = tab.sort_descending;
            reopened.folders_position = tab.folders_position;
            reopened.show_left_sidebar = tab.show_left_sidebar;
            reopened.show_preview_panel = tab.show_preview_panel;
            reopened.sidebar_expanded = tab.sidebar_expanded;
            reopened.sidebar_scroll_y = tab.sidebar_scroll_y;

            self.next_id += 1;

            // Insert after active tab
            let insert_pos = self.active_tab + 1;
            self.tabs.insert(insert_pos, reopened);
            self.active_tab = insert_pos;
            true
        } else {
            false
        }
    }

    /// Get number of open tabs
    pub fn count(&self) -> usize {
        self.tabs.len()
    }
}
