mod background_jobs;
pub(crate) mod consistency_probe_worker;
mod filesystem_workers;
mod pipeline_workers;
mod visual_workers;

pub(super) use background_jobs::{spawn_incremental_gc_worker, spawn_startup_drive_info_preload};
pub(crate) use background_jobs::stop_gc_worker;
pub(super) use consistency_probe_worker::spawn_consistency_probe_worker;
pub(super) use filesystem_workers::{
    spawn_disk_cache_invalidation_worker, spawn_folder_preview_workers, spawn_folder_size_worker,
};
pub(crate) use filesystem_workers::CacheInvalidationEntry;
pub(super) use pipeline_workers::{
    spawn_file_operation_worker, spawn_global_search_worker, spawn_prefetching_workers,
    PrefetchWorkerHandles,
};
pub(super) use visual_workers::{
    spawn_async_font_loader, spawn_cover_worker, spawn_icon_worker,
    spawn_live_file_size_worker, spawn_metadata_worker,
};
