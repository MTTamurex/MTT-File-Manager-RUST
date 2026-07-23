//! Dual panel state types.
//!
//! Defines the data model for the dual-panel (split view) mode.
//! The active panel's state lives in `ImageViewerApp`'s existing fields.
//! The inactive panel's state is stored in a `PanelSnapshot`.

use crate::application::navigation::NavigationHistory;
use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode, ViewMode};
use crate::ui::cache::FxHashSet;

use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use super::layout_state::LayoutState;

/// Which panel is currently active (receives keyboard/sidebar input).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActivePanel {
    Left,
    Right,
}

impl ActivePanel {
    /// Returns the opposite panel.
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Per-panel list-view column widths.
///
/// `ImageViewerApp::layout` stores the active panel widths. Dual-panel snapshots
/// keep a copy for the inactive panel so resizing a list column in one panel
/// does not mutate the other panel.
#[derive(Clone, Copy)]
pub struct PanelListColumnWidths {
    pub name: f32,
    pub date: f32,
    pub type_col: f32,
    pub size: f32,
    pub onedrive_name: f32,
    pub onedrive_date: f32,
    pub onedrive_type: f32,
    pub onedrive_size: f32,
    pub onedrive_status: f32,
    pub computer_name: f32,
    pub computer_total: f32,
    pub computer_free: f32,
}

impl PanelListColumnWidths {
    pub(crate) fn from_layout(layout: &LayoutState) -> Self {
        Self {
            name: layout.list_col_name_width,
            date: layout.list_col_date_width,
            type_col: layout.list_col_type_width,
            size: layout.list_col_size_width,
            onedrive_name: layout.list_col_onedrive_name_width,
            onedrive_date: layout.list_col_onedrive_date_width,
            onedrive_type: layout.list_col_onedrive_type_width,
            onedrive_size: layout.list_col_onedrive_size_width,
            onedrive_status: layout.list_col_onedrive_status_width,
            computer_name: layout.list_col_computer_name_width,
            computer_total: layout.list_col_computer_total_width,
            computer_free: layout.list_col_computer_free_width,
        }
    }

    pub(crate) fn apply_to_layout(self, layout: &mut LayoutState) {
        layout.list_col_name_width = self.name;
        layout.list_col_date_width = self.date;
        layout.list_col_type_width = self.type_col;
        layout.list_col_size_width = self.size;
        layout.list_col_onedrive_name_width = self.onedrive_name;
        layout.list_col_onedrive_date_width = self.onedrive_date;
        layout.list_col_onedrive_type_width = self.onedrive_type;
        layout.list_col_onedrive_size_width = self.onedrive_size;
        layout.list_col_onedrive_status_width = self.onedrive_status;
        layout.list_col_computer_name_width = self.computer_name;
        layout.list_col_computer_total_width = self.computer_total;
        layout.list_col_computer_free_width = self.computer_free;
    }

    fn swap_with_layout(&mut self, layout: &mut LayoutState) {
        std::mem::swap(&mut self.name, &mut layout.list_col_name_width);
        std::mem::swap(&mut self.date, &mut layout.list_col_date_width);
        std::mem::swap(&mut self.type_col, &mut layout.list_col_type_width);
        std::mem::swap(&mut self.size, &mut layout.list_col_size_width);
        std::mem::swap(
            &mut self.onedrive_name,
            &mut layout.list_col_onedrive_name_width,
        );
        std::mem::swap(
            &mut self.onedrive_date,
            &mut layout.list_col_onedrive_date_width,
        );
        std::mem::swap(
            &mut self.onedrive_type,
            &mut layout.list_col_onedrive_type_width,
        );
        std::mem::swap(
            &mut self.onedrive_size,
            &mut layout.list_col_onedrive_size_width,
        );
        std::mem::swap(
            &mut self.onedrive_status,
            &mut layout.list_col_onedrive_status_width,
        );
        std::mem::swap(
            &mut self.computer_name,
            &mut layout.list_col_computer_name_width,
        );
        std::mem::swap(
            &mut self.computer_total,
            &mut layout.list_col_computer_total_width,
        );
        std::mem::swap(
            &mut self.computer_free,
            &mut layout.list_col_computer_free_width,
        );
    }
}

/// Snapshot of one panel's browsing state.
///
/// This mirrors the per-panel fields of `ImageViewerApp` so that the inactive
/// panel can be stored separately while the active panel occupies the main
/// app fields (zero change to grid/list view rendering code).
#[derive(Clone)]
pub struct PanelSnapshot {
    // Navigation
    pub path: String,
    pub path_input: String,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub navigation: NavigationHistory,

    // Content
    pub items: Arc<Vec<FileEntry>>,
    pub all_items: Arc<Vec<FileEntry>>,
    pub items_snapshot_compact: bool,
    pub total_items: usize,
    pub is_loading_folder: bool,
    pub folder_load_error: Option<crate::app::state::FolderLoadError>,

    // Selection
    pub selected_item: Option<usize>,
    pub selected_file: Option<FileEntry>,
    pub multi_selection: FxHashSet<PathBuf>,
    pub selection_anchor: Option<usize>,
    pub rectangle_selection_state:
        Option<crate::ui::views::rectangle_selection::RectangleSelectionState>,

    // View preferences
    pub view_mode: ViewMode,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub folders_position: FoldersPosition,
    pub current_folder_locked: bool,
    pub list_column_widths: PanelListColumnWidths,
    pub miller_columns: crate::app::miller_columns_state::MillerColumnsState,

    // Scroll
    pub scroll_offset_y: f32,
    pub scroll_offset_x: f32,
    pub scroll_to_selected: bool,
    pub visible_index_range: Option<(usize, usize)>,
    pub visible_paths_cache: FxHashSet<PathBuf>,
    pub visible_range_cached: Option<(usize, usize)>,

    // Search
    pub search_query: String,
    pub active_tag_filter: Option<i64>,

    // Preview cache
    pub selected_thumbnail: Option<egui::TextureHandle>,
    pub selected_metadata: Option<(PathBuf, crate::infrastructure::windows::MediaMetadata)>,
    pub selected_gif: Option<crate::ui::components::media_preview::GifPlayer>,

    // Loaded path tracker (prevents duplicate loads)
    pub loaded_path: String,

    // Rename state
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,

    // Per-panel async loading pipeline state
    pub generation: usize,
    pub current_generation: Arc<AtomicUsize>,
    pub pending_all_items_clear: bool,
    pub hold_visible_items_until_load_complete: bool,
    pub pending_items_rebuild: bool,
    pub pending_items_count: usize,
    pub loading_started_at: Instant,
    pub last_items_rebuild: Instant,
    pub stale_items_snapshot: Option<std::collections::HashMap<PathBuf, (u64, u64)>>,
}

impl PanelSnapshot {
    pub(crate) fn compact_for_storage(&mut self) {
        self.compact_items_snapshot();
        self.selected_thumbnail = None;
        self.selected_gif = None;
    }

    pub(crate) fn restore_from_storage(&mut self) {
        self.restore_items_snapshot();
    }

    fn can_compact_items_snapshot(&self) -> bool {
        self.search_query.is_empty()
            && !self.is_loading_folder
            && !self.pending_items_rebuild
            && self.items.len() == self.all_items.len()
    }

    fn compact_items_snapshot(&mut self) {
        if self.can_compact_items_snapshot() {
            self.all_items = self.items.clone();
            self.items = Arc::new(Vec::new());
            self.items_snapshot_compact = true;
        } else {
            self.items_snapshot_compact = false;
        }
    }

    fn restore_items_snapshot(&mut self) {
        if self.items_snapshot_compact {
            self.items = self.all_items.clone();
            self.items_snapshot_compact = false;
        }
    }

    /// Capture the current app state into a snapshot.
    pub fn from_app(app: &crate::app::state::ImageViewerApp) -> Self {
        Self {
            path: app.navigation_state.current_path.clone(),
            path_input: app.navigation_state.path_input.clone(),
            is_computer_view: app.navigation_state.is_computer_view,
            is_recycle_bin_view: app.navigation_state.is_recycle_bin_view,
            navigation: app.navigation_state.navigation.clone(),
            items: app.items.clone(),
            all_items: app.all_items.clone(),
            items_snapshot_compact: false,
            total_items: app.total_items,
            is_loading_folder: app.is_loading_folder,
            folder_load_error: app.folder_load_error.clone(),
            selected_item: app.selected_item,
            selected_file: app.selected_file.clone(),
            multi_selection: app.multi_selection.clone(),
            selection_anchor: app.selection_anchor,
            rectangle_selection_state: app.rectangle_selection_state.clone(),
            view_mode: app.view_mode,
            sort_mode: app.sort_mode,
            sort_descending: app.sort_descending,
            folders_position: app.folders_position,
            current_folder_locked: app.current_folder_locked,
            list_column_widths: PanelListColumnWidths::from_layout(&app.layout),
            miller_columns: app.miller_columns.clone(),
            scroll_offset_y: app.scroll_offset_y,
            scroll_offset_x: app.scroll_offset_x,
            scroll_to_selected: app.scroll_to_selected,
            visible_index_range: app.visible_index_range,
            visible_paths_cache: app.visible_paths_cache.clone(),
            visible_range_cached: app.visible_range_cached,
            search_query: app.search_query.clone(),
            active_tag_filter: app.active_tag_filter,
            selected_thumbnail: app.selected_thumbnail.clone(),
            selected_metadata: app.selected_metadata.clone(),
            selected_gif: app.selected_gif.clone(),
            loaded_path: app.loaded_path.clone(),
            renaming_state: app.renaming_state.clone(),
            focus_rename: app.focus_rename,
            generation: app.generation,
            current_generation: app.current_generation.clone(),
            pending_all_items_clear: app.pending_all_items_clear,
            hold_visible_items_until_load_complete: app.hold_visible_items_until_load_complete,
            pending_items_rebuild: app.pending_items_rebuild,
            pending_items_count: app.pending_items_count,
            loading_started_at: app.loading_started_at,
            last_items_rebuild: app.last_items_rebuild,
            stale_items_snapshot: app.stale_items_snapshot.clone(),
        }
    }

    /// Restore this snapshot into the app's main fields.
    pub fn apply_to(mut self, app: &mut crate::app::state::ImageViewerApp) {
        self.restore_items_snapshot();
        app.navigation_state.current_path = self.path;
        app.navigation_state.path_input = self.path_input;
        app.navigation_state.is_computer_view = self.is_computer_view;
        app.navigation_state.is_recycle_bin_view = self.is_recycle_bin_view;
        app.navigation_state.navigation = self.navigation;
        app.items = self.items;
        app.all_items = self.all_items;
        app.total_items = self.total_items;
        app.is_loading_folder = self.is_loading_folder;
        app.folder_load_error = self.folder_load_error;
        app.selected_item = self.selected_item;
        app.selected_file = self.selected_file;
        app.multi_selection = self.multi_selection;
        app.selection_anchor = self.selection_anchor;
        app.rectangle_selection_state = self.rectangle_selection_state;
        app.view_mode = self.view_mode;
        app.sort_mode = self.sort_mode;
        app.sort_descending = self.sort_descending;
        app.folders_position = self.folders_position;
        app.current_folder_locked = self.current_folder_locked;
        self.list_column_widths.apply_to_layout(&mut app.layout);
        app.miller_columns = self.miller_columns;
        app.scroll_offset_y = self.scroll_offset_y;
        app.scroll_offset_x = self.scroll_offset_x;
        app.scroll_to_selected = self.scroll_to_selected;
        app.visible_index_range = self.visible_index_range;
        app.visible_paths_cache = self.visible_paths_cache;
        app.visible_range_cached = self.visible_range_cached;
        app.search_query = self.search_query;
        app.active_tag_filter = self.active_tag_filter;
        app.selected_thumbnail = self.selected_thumbnail;
        app.selected_metadata = self.selected_metadata;
        app.selected_gif = self.selected_gif;
        app.loaded_path = self.loaded_path;
        app.renaming_state = self.renaming_state;
        app.focus_rename = self.focus_rename;
        app.generation = self.generation;
        app.current_generation = self.current_generation;
        app.pending_all_items_clear = self.pending_all_items_clear;
        app.hold_visible_items_until_load_complete = self.hold_visible_items_until_load_complete;
        app.pending_items_rebuild = self.pending_items_rebuild;
        app.pending_items_count = self.pending_items_count;
        app.loading_started_at = self.loading_started_at;
        app.last_items_rebuild = self.last_items_rebuild;
        app.stale_items_snapshot = self.stale_items_snapshot;
    }

    /// Zero-allocation swap: exchange every field between `self` and the app's
    /// main fields using `std::mem::swap`. No clones, no allocations.
    pub fn swap_with_app(&mut self, app: &mut crate::app::state::ImageViewerApp) {
        self.restore_items_snapshot();
        std::mem::swap(&mut self.path, &mut app.navigation_state.current_path);
        std::mem::swap(&mut self.path_input, &mut app.navigation_state.path_input);
        std::mem::swap(
            &mut self.is_computer_view,
            &mut app.navigation_state.is_computer_view,
        );
        std::mem::swap(
            &mut self.is_recycle_bin_view,
            &mut app.navigation_state.is_recycle_bin_view,
        );
        std::mem::swap(&mut self.navigation, &mut app.navigation_state.navigation);
        std::mem::swap(&mut self.items, &mut app.items);
        std::mem::swap(&mut self.all_items, &mut app.all_items);
        std::mem::swap(&mut self.total_items, &mut app.total_items);
        std::mem::swap(&mut self.is_loading_folder, &mut app.is_loading_folder);
        std::mem::swap(&mut self.folder_load_error, &mut app.folder_load_error);
        std::mem::swap(&mut self.selected_item, &mut app.selected_item);
        std::mem::swap(&mut self.selected_file, &mut app.selected_file);
        std::mem::swap(&mut self.multi_selection, &mut app.multi_selection);
        std::mem::swap(&mut self.selection_anchor, &mut app.selection_anchor);
        std::mem::swap(
            &mut self.rectangle_selection_state,
            &mut app.rectangle_selection_state,
        );
        std::mem::swap(&mut self.view_mode, &mut app.view_mode);
        std::mem::swap(&mut self.sort_mode, &mut app.sort_mode);
        std::mem::swap(&mut self.sort_descending, &mut app.sort_descending);
        std::mem::swap(&mut self.folders_position, &mut app.folders_position);
        std::mem::swap(
            &mut self.current_folder_locked,
            &mut app.current_folder_locked,
        );
        self.list_column_widths.swap_with_layout(&mut app.layout);
        std::mem::swap(&mut self.miller_columns, &mut app.miller_columns);
        std::mem::swap(&mut self.scroll_offset_y, &mut app.scroll_offset_y);
        std::mem::swap(&mut self.scroll_offset_x, &mut app.scroll_offset_x);
        std::mem::swap(&mut self.scroll_to_selected, &mut app.scroll_to_selected);
        std::mem::swap(&mut self.visible_index_range, &mut app.visible_index_range);
        std::mem::swap(&mut self.visible_paths_cache, &mut app.visible_paths_cache);
        std::mem::swap(
            &mut self.visible_range_cached,
            &mut app.visible_range_cached,
        );
        std::mem::swap(&mut self.search_query, &mut app.search_query);
        std::mem::swap(&mut self.active_tag_filter, &mut app.active_tag_filter);
        std::mem::swap(&mut self.selected_thumbnail, &mut app.selected_thumbnail);
        std::mem::swap(&mut self.selected_metadata, &mut app.selected_metadata);
        std::mem::swap(&mut self.selected_gif, &mut app.selected_gif);
        std::mem::swap(&mut self.loaded_path, &mut app.loaded_path);
        std::mem::swap(&mut self.renaming_state, &mut app.renaming_state);
        std::mem::swap(&mut self.focus_rename, &mut app.focus_rename);
        std::mem::swap(&mut self.generation, &mut app.generation);
        std::mem::swap(&mut self.current_generation, &mut app.current_generation);
        std::mem::swap(
            &mut self.pending_all_items_clear,
            &mut app.pending_all_items_clear,
        );
        std::mem::swap(
            &mut self.hold_visible_items_until_load_complete,
            &mut app.hold_visible_items_until_load_complete,
        );
        std::mem::swap(
            &mut self.pending_items_rebuild,
            &mut app.pending_items_rebuild,
        );
        std::mem::swap(&mut self.pending_items_count, &mut app.pending_items_count);
        std::mem::swap(&mut self.loading_started_at, &mut app.loading_started_at);
        std::mem::swap(&mut self.last_items_rebuild, &mut app.last_items_rebuild);
        std::mem::swap(
            &mut self.stale_items_snapshot,
            &mut app.stale_items_snapshot,
        );
    }
}
