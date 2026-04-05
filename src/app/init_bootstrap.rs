use crate::domain::file_entry::DriveInfo;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::folder_compose::FolderComposer;
use crate::infrastructure::icon_disk_cache::IconDiskCache;
use crate::infrastructure::onedrive;
use crate::infrastructure::windows as windows_infra;
use crate::workers::thumbnail::{
    new_shared_bulk_thumbnail_progress, spawn_thumbnail_workers, PriorityThumbnailQueue,
    SharedBulkThumbnailProgress,
};
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{mpsc, Arc};

use super::folder_size_state::FolderSizeMessage;
use super::init_preferences::StartupPreferences;
use super::init_workers::{
    spawn_async_font_loader, spawn_consistency_probe_worker, spawn_cover_worker,
    spawn_disk_cache_invalidation_worker, spawn_file_operation_worker,
    spawn_folder_preview_workers, spawn_folder_size_worker, spawn_global_search_worker,
    spawn_icon_worker, spawn_live_file_size_worker, spawn_metadata_worker,
    spawn_prefetching_workers, PrefetchWorkerHandles,
};
use super::state::ItemsRebuildResult;

pub(in crate::app) struct AppBootstrap {
    pub(in crate::app) file_entry_sender: mpsc::Sender<(usize, Vec<FileEntry>)>,
    pub(in crate::app) file_entry_receiver: mpsc::Receiver<(usize, Vec<FileEntry>)>,
    pub(in crate::app) items_rebuild_sender: mpsc::Sender<ItemsRebuildResult>,
    pub(in crate::app) items_rebuild_receiver: mpsc::Receiver<ItemsRebuildResult>,

    pub(in crate::app) disk_cache: Arc<ThumbnailDiskCache>,
    pub(in crate::app) directory_index: Option<Arc<DirectoryIndex>>,
    pub(in crate::app) directory_cache: Arc<DirectoryCache>,
    pub(in crate::app) startup_preferences: StartupPreferences,

    pub(in crate::app) cover_req_tx: mpsc::Sender<PathBuf>,
    pub(in crate::app) cover_res_rx: mpsc::Receiver<(PathBuf, Option<PathBuf>)>,
    #[cfg(feature = "notify-watcher")]
    pub(in crate::app) fs_tx: mpsc::Sender<notify::Result<notify::Event>>,
    #[cfg(feature = "notify-watcher")]
    pub(in crate::app) fs_rx: mpsc::Receiver<notify::Result<notify::Event>>,
    pub(in crate::app) device_event_receiver: mpsc::Receiver<()>,

    pub(in crate::app) thumbnail_queue: Arc<PriorityThumbnailQueue>,
    pub(in crate::app) shared_gen: Arc<AtomicUsize>,
    pub(in crate::app) img_rx: crossbeam_channel::Receiver<crate::domain::thumbnail::ThumbnailData>,
    pub(in crate::app) pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,
    pub(in crate::app) bulk_thumbnail_progress: SharedBulkThumbnailProgress,
    pub(in crate::app) bulk_thumbnail_scanning: Arc<AtomicBool>,
    pub(in crate::app) bulk_thumbnail_total: Arc<AtomicUsize>,
    pub(in crate::app) bulk_thumbnail_completed: Arc<AtomicUsize>,
    pub(in crate::app) font_rx: mpsc::Receiver<egui::FontDefinitions>,

    pub(in crate::app) icon_req_tx: mpsc::Sender<(PathBuf, usize)>,
    pub(in crate::app) icon_res_rx: mpsc::Receiver<(PathBuf, usize, Vec<u8>, u32, u32)>,
    pub(in crate::app) meta_req_tx: mpsc::Sender<(PathBuf, u64)>,
    pub(in crate::app) meta_res_rx: mpsc::Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
    pub(in crate::app) live_size_req_tx: mpsc::Sender<super::live_file_size::LiveFileSizeRequest>,
    pub(in crate::app) live_size_res_rx:
        mpsc::Receiver<super::live_file_size::LiveFileSizeResponse>,
    pub(in crate::app) folder_preview_tx: crossbeam_channel::Sender<PathBuf>,
    pub(in crate::app) folder_preview_res_rx:
        mpsc::Receiver<crate::workers::folder_preview_worker::FolderPreviewData>,
    pub(in crate::app) folder_size_req_tx: mpsc::Sender<PathBuf>,
    pub(in crate::app) folder_size_res_rx: mpsc::Receiver<FolderSizeMessage>,
    pub(in crate::app) folder_size_cancel: Arc<AtomicBool>,

    pub(in crate::app) prefetch_tx: mpsc::Sender<crate::workers::prefetch_worker::PrefetchMessage>,
    pub(in crate::app) idle_warmup_tx: mpsc::Sender<crate::workers::idle_warmup::IdleWarmupMessage>,

    pub(in crate::app) file_op_tx:
        mpsc::Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    pub(in crate::app) file_op_res_rx:
        mpsc::Receiver<crate::workers::file_operation_worker::FileOperationResult>,
    pub(in crate::app) extraction_progress:
        crate::infrastructure::archive_extract::SharedExtractionProgress,
    pub(in crate::app) global_search_tx:
        mpsc::Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
    pub(in crate::app) global_search_res_rx:
        mpsc::Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
    pub(in crate::app) disk_cache_invalidation_tx:
        mpsc::Sender<Vec<crate::app::init_workers::CacheInvalidationEntry>>,

    pub(in crate::app) consistency_probe_tx:
        mpsc::Sender<super::init_workers::consistency_probe_worker::ConsistencyProbeRequest>,
    pub(in crate::app) consistency_probe_rx:
        mpsc::Receiver<super::init_workers::consistency_probe_worker::ConsistencyProbeResult>,

    pub(in crate::app) disks: Vec<(String, String)>,
    pub(in crate::app) drive_scan_tx: mpsc::Sender<Vec<(String, String)>>,
    pub(in crate::app) drive_scan_rx: mpsc::Receiver<Vec<(String, String)>>,
    pub(in crate::app) drive_info_tx: mpsc::Sender<Vec<(String, DriveInfo)>>,
    pub(in crate::app) drive_info_rx: mpsc::Receiver<Vec<(String, DriveInfo)>>,

    /// Pre-loaded extension icon pixel data from disk cache.
    /// Key = extension (e.g. "dll"), Value = (rgba_pixels, width, height).
    pub(in crate::app) preloaded_extension_icons: HashMap<String, (Vec<u8>, u32, u32)>,

    /// Custom composed empty folder icon (back+front+paper_sheet).
    /// Used as the default folder icon instead of the Windows yellow folder.
    pub(in crate::app) custom_folder_icon: (Vec<u8>, u32, u32),
}

pub(in crate::app) fn bootstrap_app(ctx: &egui::Context) -> AppBootstrap {
    const THUMBNAIL_RESULT_CHANNEL_CAPACITY: usize = 2048;

    if let Err(error) = crate::infrastructure::virtual_drive_config::ensure_config_exists() {
        log::warn!(
            "[Config] Failed to initialize virtual drive configuration: {}",
            error
        );
    }

    let (file_entry_sender, file_entry_receiver) = mpsc::channel::<(usize, Vec<FileEntry>)>();
    let (items_rebuild_sender, items_rebuild_receiver) = mpsc::channel::<ItemsRebuildResult>();

    let cache_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MTT-File-Manager")
        .join("thumbnails");
    let disk_cache = Arc::new(match ThumbnailDiskCache::new(cache_dir.clone()) {
        Ok(cache) => cache,
        Err(e) => {
            log::error!(
                "[Cache] Failed to initialize thumbnail cache at {:?}: {:?}. Retrying in-memory.",
                cache_dir,
                e
            );
            // Last-resort in-memory fallback — thumbnails won't persist but app keeps running.
            ThumbnailDiskCache::new(std::env::temp_dir().join("mtt-cache-fallback"))
                .unwrap_or_else(|e2| {
                    log::error!("[Cache] In-memory fallback also failed: {:?}. Exiting.", e2);
                    std::process::exit(1);
                })
        }
    });
    let directory_index = match DirectoryIndex::open(&cache_dir.join("thumbnails.db")) {
        Ok(index) => Some(Arc::new(index)),
        Err(e) => {
            log::warn!("[Cache] Failed to open directory index: {:?}", e);
            None
        }
    };

    let (cover_req_tx, cover_res_rx) = spawn_cover_worker(disk_cache.clone());
    #[cfg(feature = "notify-watcher")]
    let (fs_tx, fs_rx) = mpsc::channel();
    let (device_event_sender, device_event_receiver) = mpsc::channel();
    windows_infra::start_device_change_listener(device_event_sender, ctx.clone());

    let (img_tx, img_rx) = crossbeam_channel::bounded(THUMBNAIL_RESULT_CHANNEL_CAPACITY);
    let thumbnail_queue = Arc::new(PriorityThumbnailQueue::new());
    let shared_gen = Arc::new(AtomicUsize::new(0));
    let bulk_thumbnail_progress = new_shared_bulk_thumbnail_progress();
    let bulk_thumbnail_scanning = Arc::new(AtomicBool::new(false));
    let bulk_thumbnail_total = Arc::new(AtomicUsize::new(0));
    let bulk_thumbnail_completed = Arc::new(AtomicUsize::new(0));

    onedrive::init_onedrive_paths();
    let directory_cache = Arc::new(DirectoryCache::new());
    let startup_preferences = StartupPreferences::load(&disk_cache);
    let font_rx = spawn_async_font_loader();

    let pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>> = Arc::new(dashmap::DashMap::new());
    spawn_thumbnail_workers(
        thumbnail_queue.clone(),
        img_tx,
        ctx.clone(),
        shared_gen.clone(),
        disk_cache.clone(),
        pending_deletions.clone(),
        bulk_thumbnail_progress.clone(),
        bulk_thumbnail_completed.clone(),
    );

    // --- Icon disk cache: persist extension icons across app launches ---
    let app_data_dir = cache_dir
        .parent()
        .unwrap_or(&cache_dir)
        .to_path_buf();
    let icon_disk_cache = Arc::new(IconDiskCache::new(&app_data_dir));
    let preloaded_extension_icons = icon_disk_cache.load_all();

    let (icon_req_tx, icon_res_rx) =
        spawn_icon_worker(ctx, shared_gen.clone(), icon_disk_cache, &preloaded_extension_icons);

    // Pre-warm: only send requests for extensions NOT already in disk cache.
    // Disk-cached extensions are loaded instantly in ImageViewerApp::new().
    {
        const PREWARM: &[&str] = &[
            // C:\Windows visible extensions (highest priority — first in queue)
            "dll", "sys", "log", "txt", "ini", "xml", "bat", "bin",
            "exe", "cmd", "dat", "prx", "cat", "mun", "nls", "inf",
            // Common document/code extensions
            "pdf", "json", "html", "css", "js", "md", "csv", "rtf",
            "doc", "docx", "xls", "xlsx", "ppt", "pptx",
            // Archives
            "zip", "rar", "7z", "tar", "gz",
            // Images
            "jpg", "png", "gif", "bmp", "svg", "ico", "webp",
            // Media
            "mp3", "mp4", "mkv", "avi", "wav", "flac",
            // Code
            "py", "rs", "cpp", "c", "h", "java", "cs", "go", "ts",
            "toml", "yaml", "cfg", "reg", "msi", "cab", "tmp",
        ];
        let mut prewarm_skipped = 0usize;
        for ext in PREWARM {
            if preloaded_extension_icons.contains_key(*ext) {
                prewarm_skipped += 1;
                continue; // Already in disk cache — will be loaded instantly.
            }
            let fake = PathBuf::from(format!("__prewarm__\\file.{}", ext));
            let _ = icon_req_tx.send((fake, usize::MAX));
        }
        if prewarm_skipped > 0 {
            log::info!(
                "[IconPrewarm] Skipped {} extensions (disk-cached), sending {} to workers",
                prewarm_skipped,
                PREWARM.len() - prewarm_skipped,
            );
        }
    }

    let (meta_req_tx, meta_res_rx) = spawn_metadata_worker(ctx);
    let (live_size_req_tx, live_size_res_rx) = spawn_live_file_size_worker(ctx);
    let folder_composer = Arc::new(FolderComposer::new());
    // Compose the custom empty folder icon ONCE before sharing the composer.
    let custom_folder_icon = folder_composer.compose_empty();
    let (folder_preview_tx, folder_preview_res_rx) =
        spawn_folder_preview_workers(ctx, disk_cache.clone(), folder_composer);
    let (folder_size_req_tx, folder_size_res_rx, folder_size_cancel) =
        spawn_folder_size_worker(ctx);

    let PrefetchWorkerHandles {
        prefetch_sender: prefetch_tx,
        idle_warmup_sender: idle_warmup_tx,
    } = spawn_prefetching_workers(
        directory_cache.clone(),
        thumbnail_queue.clone(),
        shared_gen.clone(),
    );

    let (file_op_tx, file_op_res_rx, extraction_progress) = spawn_file_operation_worker();
    let (global_search_tx, global_search_res_rx) = spawn_global_search_worker(ctx);
    let disk_cache_invalidation_tx = spawn_disk_cache_invalidation_worker(disk_cache.clone());
    let (consistency_probe_tx, consistency_probe_rx) = spawn_consistency_probe_worker(ctx.clone());

    let disks = windows_infra::get_all_drives();
    let (drive_scan_tx, drive_scan_rx) = mpsc::channel();
    let (drive_info_tx, drive_info_rx) = mpsc::channel();

    AppBootstrap {
        file_entry_sender,
        file_entry_receiver,
        items_rebuild_sender,
        items_rebuild_receiver,
        disk_cache,
        directory_index,
        directory_cache,
        startup_preferences,
        cover_req_tx,
        cover_res_rx,
        #[cfg(feature = "notify-watcher")]
        fs_tx,
        #[cfg(feature = "notify-watcher")]
        fs_rx,
        device_event_receiver,
        thumbnail_queue,
        shared_gen,
        img_rx,
        pending_deletions,
        bulk_thumbnail_progress,
        bulk_thumbnail_scanning,
        bulk_thumbnail_total,
        bulk_thumbnail_completed,
        font_rx,
        icon_req_tx,
        icon_res_rx,
        meta_req_tx,
        meta_res_rx,
        live_size_req_tx,
        live_size_res_rx,
        folder_preview_tx,
        folder_preview_res_rx,
        folder_size_req_tx,
        folder_size_res_rx,
        folder_size_cancel,
        prefetch_tx,
        idle_warmup_tx,
        file_op_tx,
        file_op_res_rx,
        extraction_progress,
        global_search_tx,
        global_search_res_rx,
        disk_cache_invalidation_tx,
        consistency_probe_tx,
        consistency_probe_rx,
        disks,
        drive_scan_tx,
        drive_scan_rx,
        drive_info_tx,
        drive_info_rx,
        preloaded_extension_icons,
        custom_folder_icon,
    }
}
