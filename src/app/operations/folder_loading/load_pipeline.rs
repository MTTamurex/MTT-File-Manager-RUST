use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::adaptive_batch::{AdaptiveBatchConfig, AdaptiveBatchTracker};
use crate::infrastructure::io_priority;
use crate::infrastructure::onedrive;
mod fast_paths;
mod optimized_tiers;
mod tier3_fallback;

use std::path::PathBuf;

impl ImageViewerApp {
    pub(super) fn start_folder_load_pipeline(
        &mut self,
        force_refresh: bool,
        validate_against_current_generation: bool,
    ) {
        let my_gen = self.generation;
        let gen_clone = if validate_against_current_generation {
            self.current_generation.clone()
        } else {
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(my_gen))
        };

        let job = crate::app::init_workers::folder_load_pool::FolderLoadJob {
            lane: if validate_against_current_generation {
                crate::app::init_workers::folder_load_pool::FolderLoadLane::Active
            } else {
                crate::app::init_workers::folder_load_pool::FolderLoadLane::Inactive
            },
            my_gen,
            gen_clone,
            current_path: self.navigation_state.current_path.clone(),
            force_refresh,
            file_entry_sender: self.file_entry_sender.clone(),
            folder_load_failure_sender: self.folder_load_failure_sender.clone(),
            ctx: self.ui_ctx.clone(),
            disk_cache: self.disk_cache.clone(),
            app_state_db: self.app_state_db.clone(),
            directory_cache: self.directory_cache.clone(),
            directory_dirty_registry: self.directory_dirty_registry.clone(),
            directory_index_opt: self.directory_index.clone(),
            show_hidden: self.show_hidden_files,
        };

        // EST-02: submit to the bounded persistent pool instead of spawning a
        // detached thread per navigation. Stale jobs exit instantly via the
        // generation guard; workers blocked in kernel I/O cap thread growth
        // at the pool size instead of accumulating one thread per navigation.
        self.folder_load_pool.submit(job);
    }
}

/// EST-02: folder-load pipeline body, executed by the pool workers.
pub(crate) fn run_folder_load_pipeline(
    job: crate::app::init_workers::folder_load_pool::FolderLoadJob,
) {
    let crate::app::init_workers::folder_load_pool::FolderLoadJob {
        lane: _,
        my_gen,
        gen_clone,
        current_path,
        force_refresh,
        file_entry_sender,
        folder_load_failure_sender,
        ctx,
        disk_cache,
        app_state_db,
        directory_cache,
        directory_dirty_registry,
        directory_index_opt,
        show_hidden,
    } = job;

    // Immediate generation check: if a newer load was already started
    // before this job was picked up, exit instantly without doing
    // any I/O. This prevents stale work from rapid navigation.
    if gen_clone.load(std::sync::atomic::Ordering::Relaxed) != my_gen {
        return;
    }

    let scan_start = std::time::Instant::now();

    let base_path = if current_path.len() == 2 && current_path.ends_with(':') {
        format!("{}\\", current_path)
    } else {
        current_path.clone()
    };

    let is_ssd = io_priority::is_ssd(&PathBuf::from(&current_path));
    let config = AdaptiveBatchConfig {
        is_ssd,
        total_items: directory_index_opt
            .as_ref()
            .and_then(|di| di.get_directory(&PathBuf::from(&base_path)))
            .map(|(meta, _)| meta.file_count),
    };
    let mut batch_tracker = AdaptiveBatchTracker::new(config);
    let mut batch_size = batch_tracker.batch_size();

    // STALE-WHILE-REVALIDATE STRATEGY: Instant feedback via DirectoryCache
    let base_path_buf = PathBuf::from(&base_path);
    // PERFORMANCE: Only use is_cloud_sync_path() which is string-based (no I/O)
    // path_has_cloud_attributes() was removed because GetFileAttributesW can BLOCK
    // indefinitely on cloud-only provider folders
    let is_onedrive_base = onedrive::is_cloud_sync_path(&base_path_buf);
    let prefer_reliable_scan = directory_dirty_registry.is_dirty(base_path_buf.as_path())
        || (!is_onedrive_base
            && !crate::infrastructure::windows::path_is_usn_filesystem(base_path_buf.as_path())
                .unwrap_or(true));
    let mut batch = Vec::with_capacity(batch_size);
    let mut all_entries_disk: Vec<FileEntry> = Vec::new();
    let mut batch_start = std::time::Instant::now();
    if fast_paths::try_handle_fast_paths(
        my_gen,
        &gen_clone,
        &current_path,
        force_refresh,
        &base_path,
        &base_path_buf,
        is_ssd,
        is_onedrive_base,
        &mut batch_size,
        &mut batch_tracker,
        &mut batch_start,
        &file_entry_sender,
        &ctx,
        &disk_cache,
        &app_state_db,
        &directory_cache,
        &directory_dirty_registry,
        &directory_index_opt,
        show_hidden,
    ) {
        return;
    }

    if optimized_tiers::try_handle_optimized_tiers(
        my_gen,
        &gen_clone,
        &scan_start,
        &base_path,
        is_ssd,
        is_onedrive_base,
        prefer_reliable_scan,
        &mut batch_size,
        &mut batch_tracker,
        &mut batch_start,
        &mut batch,
        &mut all_entries_disk,
        &file_entry_sender,
        &ctx,
        &disk_cache,
        &app_state_db,
        &directory_cache,
        &directory_dirty_registry,
        &directory_index_opt,
        show_hidden,
    ) {
        return;
    }

    tier3_fallback::run_tier3_fallback(
        my_gen,
        &gen_clone,
        &scan_start,
        &current_path,
        &base_path,
        is_onedrive_base,
        &mut batch_size,
        &mut batch_tracker,
        &mut batch_start,
        &mut batch,
        &mut all_entries_disk,
        &file_entry_sender,
        &folder_load_failure_sender,
        &ctx,
        &disk_cache,
        &app_state_db,
        &directory_cache,
        &directory_dirty_registry,
        &directory_index_opt,
        show_hidden,
    );
}
