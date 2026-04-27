use crate::domain::file_entry::DriveInfo;
use crate::infrastructure::app_state_db::AppStateDb;
use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{mpsc, Arc};

use super::drive_state::DriveState;
use super::file_operation_state::FileOperationState;
use super::folder_size_state::{FolderSizeMessage, FolderSizeState};
use super::layout_state::LayoutState;

fn load_width_pref(app_state_db: &AppStateDb, key: &str, default: f32) -> f32 {
    app_state_db
        .get_preference(key)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(default)
}

pub(in crate::app) fn build_layout_state(
    app_state_db: &AppStateDb,
    saved_window_width: f32,
    saved_window_height: f32,
    saved_is_maximized: bool,
    sidebar_left_width: f32,
    sidebar_right_width: f32,
) -> LayoutState {
    LayoutState {
        saved_window_width,
        saved_window_height,
        saved_is_maximized,
        saved_is_minimized: false,
        saved_is_fullscreen: false,
        sidebar_left_width,
        sidebar_right_width,
        list_col_name_width: load_width_pref(app_state_db, "list_col_name_width", 300.0),
        list_col_date_width: load_width_pref(app_state_db, "list_col_date_width", 170.0),
        list_col_type_width: load_width_pref(app_state_db, "list_col_type_width", 120.0),
        list_col_size_width: load_width_pref(app_state_db, "list_col_size_width", 100.0),
        list_col_onedrive_name_width: load_width_pref(
            app_state_db,
            "list_col_onedrive_name_width",
            300.0,
        ),
        list_col_onedrive_date_width: load_width_pref(
            app_state_db,
            "list_col_onedrive_date_width",
            170.0,
        ),
        list_col_onedrive_type_width: load_width_pref(
            app_state_db,
            "list_col_onedrive_type_width",
            120.0,
        ),
        list_col_onedrive_size_width: load_width_pref(
            app_state_db,
            "list_col_onedrive_size_width",
            100.0,
        ),
        list_col_onedrive_status_width: load_width_pref(
            app_state_db,
            "list_col_onedrive_status_width",
            120.0,
        ),
        list_col_computer_name_width: load_width_pref(
            app_state_db,
            "list_col_computer_name_width",
            300.0,
        ),
        list_col_computer_total_width: load_width_pref(
            app_state_db,
            "list_col_computer_total_width",
            120.0,
        ),
        list_col_computer_free_width: load_width_pref(
            app_state_db,
            "list_col_computer_free_width",
            120.0,
        ),
    }
}

pub(in crate::app) fn build_drive_state(
    disks: Vec<(String, String)>,
    drive_scan_tx: mpsc::Sender<Vec<(String, String)>>,
    drive_scan_rx: mpsc::Receiver<Vec<(String, String)>>,
    drive_info_tx: mpsc::Sender<Vec<(String, DriveInfo)>>,
    drive_info_rx: mpsc::Receiver<Vec<(String, DriveInfo)>>,
) -> DriveState {
    DriveState {
        disks,
        last_drive_refresh: std::time::Instant::now(),
        last_drive_bitmask: crate::infrastructure::windows::get_logical_drives_bitmask(),
        drive_scan_pending: false,
        drive_scan_rx,
        drive_scan_tx,
        drive_info_rx,
        drive_info_tx,
        drive_info_cache: std::collections::HashMap::new(),
    }
}

pub(in crate::app) fn build_folder_size_state(
    req_sender: mpsc::Sender<PathBuf>,
    res_receiver: mpsc::Receiver<FolderSizeMessage>,
    cancel: Arc<AtomicBool>,
    batch_req_sender: mpsc::Sender<crate::app::folder_size_state::BatchSizeRequest>,
    batch_res_receiver: mpsc::Receiver<crate::app::folder_size_state::BatchSizeResult>,
    batch_cancel: Arc<AtomicBool>,
    batch_generation: Arc<AtomicU64>,
) -> FolderSizeState {
    FolderSizeState {
        req_sender,
        res_receiver,
        cancel,
        cache: LruCache::new(
            NonZeroUsize::new(500).expect("folder_size cache size must be non-zero"),
        ),
        loading: FxHashSet::default(),
        batch_req_sender,
        batch_res_receiver,
        batch_cancel,
        batch_generation,
        batch_loading: FxHashSet::default(),
        batch_cache: LruCache::new(
            NonZeroUsize::new(2000).expect("folder_size_batch cache size must be non-zero"),
        ),
        pending_revalidation: std::collections::HashMap::new(),
        pending_revalidation_last_prune: std::time::Instant::now(),
        batch_invalidation_epoch: std::collections::HashMap::new(),
        batch_invalidation_last_prune: std::time::Instant::now(),
    }
}

pub(in crate::app) fn build_file_operation_state(
    file_op_sender: mpsc::Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    file_op_res_receiver: mpsc::Receiver<
        crate::workers::file_operation_worker::FileOperationResult,
    >,
    extraction_progress: crate::infrastructure::archive_extract::SharedExtractionProgress,
    extraction_cancel: crate::infrastructure::archive_extract::ExtractionCancelFlag,
    disk_cache_invalidation_sender: mpsc::Sender<
        Vec<crate::app::init_workers::CacheInvalidationEntry>,
    >,
    prefetch_sender: mpsc::Sender<crate::workers::prefetch_worker::PrefetchMessage>,
    idle_warmup_sender: mpsc::Sender<crate::workers::idle_warmup::IdleWarmupMessage>,
    pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,
) -> FileOperationState {
    FileOperationState {
        file_op_sender,
        file_op_res_receiver,
        extraction_progress,
        extraction_cancel,
        disk_cache_invalidation_sender,
        prefetch_sender,
        idle_warmup_sender,
        file_ops_in_progress: 0,
        pending_deletions,
        pending_iso_mount: None,
        mounted_iso_drives: std::collections::HashMap::new(),
    }
}
