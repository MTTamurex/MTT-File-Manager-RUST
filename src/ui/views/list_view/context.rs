use crate::domain::file_entry::{FileEntry, SortMode};
use crate::domain::file_tag::FileTag;
use crate::ui::cache::FxHashSet;
use crate::ui::views::rectangle_selection::{RectangleSelectionFrame, RectangleSelectionState};
use eframe::egui;
use std::path::{Path, PathBuf};

pub(crate) struct ColumnWidths {
    pub name: f32,
    pub cells: Vec<super::columns::ColumnCell>,
    pub name_offset: f32,
}

pub struct ListViewContext<'a> {
    pub(crate) columns: &'a mut crate::app::column_state::ColumnState,
    pub(crate) autofit_requested: &'a mut bool,
    pub(crate) autofit_widths: Option<Vec<(crate::domain::list_columns::ColumnId, f32)>>,
    pub(crate) autofit_visible_indices: Vec<usize>,
    pub(crate) items_revision: u64,
    pub(crate) tag_assignments_epoch: u64,
    pub items: &'a [FileEntry],
    pub group_projection: &'a crate::application::grouping::GroupProjection,
    pub collapsed_groups: &'a FxHashSet<crate::application::grouping::GroupKey>,
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub multi_selection: &'a FxHashSet<PathBuf>,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub column_sort_pending: bool,
    pub renaming_state: Option<(PathBuf, String)>,
    pub focus_rename: bool,
    pub rename_click_scope: egui::Id,
    pub rename_requested_item: Option<usize>,
    pub scroll_to_selected: bool,
    pub generation: usize,
    pub scroll_interpolation_enabled: bool,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub is_onedrive_folder: bool,
    pub global_search_active: bool,
    pub auto_resize_columns: bool,
    pub hover_preview: bool,
    pub texture_cache: &'a mut lru::LruCache<PathBuf, egui::TextureHandle>,
    pub attempted_thumbnail_bucket: &'a lru::LruCache<PathBuf, u32>,
    pub loading_set: &'a mut FxHashSet<PathBuf>,
    pub loading_icons: &'a mut crate::ui::icon_loader::IconLoadTracker,
    pub failed_icons: &'a crate::ui::icon_loader::IconFailureTracker,
    pub scanned_folders: &'a mut lru::LruCache<PathBuf, ()>,
    pub folder_icon_texture: Option<&'a egui::TextureHandle>,
    pub computer_icon: Option<&'a egui::TextureHandle>,
    pub drive_icon_cache: &'a mut lru::LruCache<String, egui::TextureHandle>,
    pub item_icon_loader: &'a mut crate::ui::icon_loader::IconLoader,
    pub deletion_date_cache: Option<&'a mut lru::LruCache<String, String>>,
    pub failed_thumbnails: &'a lru::LruCache<PathBuf, ()>,
    pub scroll_offset_y: f32,
    pub mut_scroll_offset_y: &'a mut f32,
    pub mut_scroll_offset_x: &'a mut f32,
    pub last_input: crate::app::state::LastInput,
    pub last_scroll_time: &'a mut std::time::Instant,
    pub last_scroll_offset: &'a mut f32,
    pub pending_upload_set: &'a mut FxHashSet<PathBuf>,
    pub is_on_hdd: bool,
    pub prefetch_rows: usize,
    pub visible_index_range: &'a mut Option<(usize, usize)>,
    pub visible_group_paths: &'a mut Vec<PathBuf>,
    pub is_item_dragging: bool,
    pub drag_target_folder: Option<PathBuf>,
    pub external_drop_pointer_pos: Option<egui::Pos2>,
    pub drag_started_item: &'a mut Option<usize>,
    pub drag_hovered_item: &'a mut Option<usize>,
    pub rectangle_selection_state: Option<&'a RectangleSelectionState>,
    pub rectangle_selection_frame: &'a mut RectangleSelectionFrame,
    pub live_file_size_cache: &'a mut crate::app::live_file_size::LiveFileSizeCache,
    pub live_file_size_loading: &'a mut crate::app::live_file_size::ActiveLiveFileSizeRequests,
    pub live_file_size_req_sender:
        &'a std::sync::mpsc::Sender<crate::app::live_file_size::LiveFileSizeRequest>,
    pub show_preview_panel: bool,
    pub thumbnail_requests_this_frame: usize,
    pub folder_size_cache: &'a lru::LruCache<PathBuf, u64>,
    pub folder_size_progress: &'a std::collections::HashMap<PathBuf, u64>,
    pub folder_sort_sizes: &'a std::collections::HashMap<PathBuf, u64>,
    pub folder_size_batch_loading: &'a FxHashSet<PathBuf>,
    pub folder_size_requests: &'a mut Vec<PathBuf>,
    pub col_name_width: &'a mut f32,
    pub col_date_width: &'a mut f32,
    pub col_type_width: &'a mut f32,
    pub col_size_width: &'a mut f32,
    pub col_status_width: &'a mut f32,
    pub tag_assignments: &'a rustc_hash::FxHashMap<String, Vec<i64>>,
    pub tag_definitions: &'a rustc_hash::FxHashMap<i64, FileTag>,
    /// Name-only mode used by Miller columns; does not alter the saved details layout.
    pub compact: bool,
}

pub enum ListViewAction {
    Click(usize),
    DoubleClick(usize),
    StartRename(usize),
    SecondaryClick(usize),
    SortChange(SortMode),
    EmptyAreaClick,
    EmptyAreaSecondaryClick,
    ToggleGroup(crate::application::grouping::GroupKey),
}

pub trait ListViewOperations {
    fn navigate_to(&mut self, path: &str);
    fn open_with_shell(&mut self, path: &Path);
    fn request_thumbnail_load(&mut self, path: PathBuf, directory_index: usize, modified: u64);
    fn request_thumbnail_load_with_size(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    );
    fn request_folder_scan(&mut self, path: PathBuf);
    fn request_folder_preview_load(&mut self, path: PathBuf);
    fn rename_with_shell(&mut self, idx: usize);
    fn request_thumbnail_prefetch_with_index(
        &mut self,
        path: PathBuf,
        size: u32,
        directory_index: usize,
        modified: u64,
    );
    fn request_icon_load(&mut self, path: PathBuf);
}
