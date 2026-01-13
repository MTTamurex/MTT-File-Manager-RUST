//! Main application state management
//! Follows .cursorrules: orchestration of component states, < 300 lines

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use crate::domain::file_entry::FileEntry;
use crate::infrastructure::windows;

// Import component states
use super::clipboard::{ClipboardManager, ClipboardOp};
use super::{ContextMenuState, NavigationHistory, RenamingState, WatcherState};

// Re-export for convenience
pub use crate::domain::file_entry::{SortMode, ViewMode};

/// Main application state
#[derive(Clone, Debug)]
pub struct AppState {
    // Navigation
    pub navigation: NavigationHistory,
    pub current_path: String,
    pub path_input: String,
    pub is_computer_view: bool,

    // File system
    pub items: Vec<FileEntry>,
    pub all_items: Vec<FileEntry>,
    pub total_items: usize,
    pub is_loading_folder: bool,

    // Selection
    pub selected_item_index: Option<usize>,
    pub selected_file: Option<FileEntry>,

    // Sorting and view
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub view_mode: ViewMode,
    pub thumbnail_size: f32,
    pub show_preview_panel: bool,

    // Search
    pub search_query: String,

    // Generation tracking (for async operations)
    pub generation: usize,
    pub current_generation: Arc<AtomicUsize>,

    // Component states
    pub context_menu: ContextMenuState,
    pub clipboard: ClipboardManager,
    pub watcher: WatcherState,
    pub renaming_state: Option<RenamingState>,

    // Caches (to be managed separately)
    pub scanned_folders: HashSet<PathBuf>,
    pub last_grid_cols: usize,

    // System information
    pub disks: Vec<(String, String)>,
}

impl AppState {
    /// Creates a new application state with default values
    pub fn new(initial_path: String) -> Self {
        let disks = windows::get_all_drives();

        Self {
            navigation: NavigationHistory::new(initial_path.clone()),
            current_path: initial_path.clone(),
            path_input: initial_path,
            is_computer_view: false,

            items: Vec::new(),
            all_items: Vec::new(),
            total_items: 0,
            is_loading_folder: false,

            selected_item_index: None,
            selected_file: None,

            sort_mode: SortMode::Name,
            sort_descending: false,
            view_mode: ViewMode::Grid,
            thumbnail_size: 128.0,
            show_preview_panel: true,

            search_query: String::new(),

            generation: 0,
            current_generation: Arc::new(AtomicUsize::new(0)),

            context_menu: ContextMenuState::new(),
            clipboard: ClipboardManager::new(),
            watcher: WatcherState::new(),
            renaming_state: None,

            scanned_folders: HashSet::new(),
            last_grid_cols: 1,

            disks,
        }
    }

    /// Navigates to a new path
    pub fn navigate_to(&mut self, path: &str) {
        if self.current_path == path {
            return;
        }

        self.navigation.navigate_to(path.to_string());
        self.current_path = path.to_string();
        self.path_input = path.to_string();
        self.is_computer_view = false;
        self.context_menu.target_path = None;
    }

    /// Goes back in navigation history
    pub fn go_back(&mut self) -> bool {
        if let Some(path) = self.navigation.go_back() {
            self.current_path = path.clone();
            self.path_input = path.clone();
            true
        } else {
            false
        }
    }

    /// Goes forward in navigation history
    pub fn go_forward(&mut self) -> bool {
        if let Some(path) = self.navigation.go_forward() {
            self.current_path = path.clone();
            self.path_input = path.clone();
            true
        } else {
            false
        }
    }

    /// Goes up one level
    pub fn go_up_one_level(&mut self) -> bool {
        use std::path::Path;

        if let Some(parent) = Path::new(&self.current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() {
                self.navigate_to(&parent_str);
                return true;
            }
        }
        false
    }

    /// Updates the search query and filters items
    pub fn update_search(&mut self, query: String) {
        self.search_query = query;
        self.filter_items();
    }

    /// Filters items based on search query
    pub fn filter_items(&mut self) {
        if self.search_query.is_empty() {
            self.items = self.all_items.clone();
        } else {
            let query = self.search_query.to_lowercase();
            self.items = self
                .all_items
                .iter()
                .filter(|item| item.name.to_lowercase().contains(&query))
                .cloned()
                .collect();
        }
        self.total_items = self.items.len();
    }

    /// Sorts items based on current sort mode
    pub fn sort_items(&mut self) {
        use std::cmp::Ordering;

        self.items.sort_by(|a, b| {
            // Folders always first
            if a.is_dir != b.is_dir {
                return if a.is_dir {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }

            let ordering = match self.sort_mode {
                SortMode::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortMode::Date => a.modified.cmp(&b.modified),
                SortMode::Size => a.size.cmp(&b.size),
                SortMode::Type => {
                    // Sort by file extension, then by name
                    let ext_a = a
                        .path
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    let ext_b = b
                        .path
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    match ext_a.cmp(&ext_b) {
                        std::cmp::Ordering::Equal => {
                            a.name.to_lowercase().cmp(&b.name.to_lowercase())
                        }
                        other => other,
                    }
                }
            };

            if self.sort_descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }

    /// Starts renaming an item
    pub fn start_renaming(&mut self, index: usize) {
        if let Some(item) = self.items.get(index) {
            self.renaming_state = Some(RenamingState::new(index, item.name.clone()));
        }
    }

    /// Cancels renaming
    pub fn cancel_renaming(&mut self) {
        self.renaming_state = None;
    }

    /// Completes renaming and returns the new name
    pub fn complete_renaming(&mut self) -> Option<(usize, String)> {
        self.renaming_state.take().map(|state| state.complete())
    }

    /// Updates the new name during renaming
    pub fn update_renaming_name(&mut self, new_name: String) {
        if let Some(state) = &mut self.renaming_state {
            state.update_name(new_name);
        }
    }

    /// Checks if an item is currently being renamed
    pub fn is_renaming(&self, index: usize) -> bool {
        self.renaming_state
            .as_ref()
            .map_or(false, |s| s.item_index == index)
    }

    /// Copies the selected item to clipboard
    pub fn copy_to_clipboard(&mut self) {
        if let Some(index) = self.selected_item_index {
            if let Some(item) = self.items.get(index) {
                self.clipboard.copy(&item.path);
            }
        }
    }

    /// Cuts the selected item to clipboard
    pub fn cut_to_clipboard(&mut self) {
        if let Some(index) = self.selected_item_index {
            if let Some(item) = self.items.get(index) {
                self.clipboard.cut(&item.path);
            }
        }
    }

    /// Clears the clipboard
    pub fn clear_clipboard(&mut self) {
        self.clipboard.clear();
    }

    /// Gets clipboard state for paste operation
    pub fn get_clipboard_for_paste(&self) -> Option<(&PathBuf, ClipboardOp)> {
        let (file, op) = self.clipboard.internal_state();
        file.zip(op)
    }

    /// Increments generation for async operations
    pub fn increment_generation(&mut self) {
        self.generation += 1;
        self.current_generation
            .store(self.generation, std::sync::atomic::Ordering::Relaxed);
    }

    /// Resets folder loading state
    pub fn reset_folder_state(&mut self) {
        self.items.clear();
        self.all_items.clear();
        self.scanned_folders.clear();
        self.selected_item_index = None;
        self.selected_file = None;
        self.is_loading_folder = true;
        self.total_items = 0;
    }
}
