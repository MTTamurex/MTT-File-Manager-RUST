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
    pub(super) fn start_folder_load_pipeline(&mut self, force_refresh: bool) {
        let my_gen = self.generation;
        let gen_clone = self.current_generation.clone();
        let current_path = self.navigation_state.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        let ctx = self.ui_ctx.clone();
        let disk_cache = self.disk_cache.clone();
        let directory_cache = self.directory_cache.clone();
        // Use existing directory_cache for cache-first strategy
        let directory_index_opt = self.directory_index.clone();
        let _prefetch_sender = self.file_operation_state.prefetch_sender.clone();
        let show_hidden = self.show_hidden_files;

        // STREAMING BATCH LOADING: Adaptive batch size based on disk type
        let _ = std::thread::Builder::new()
            .name("folder-load-pipeline".to_string())
            .spawn(move || {
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
            // PERFORMANCE: Only use is_onedrive_path() which is string-based (no I/O)
            // path_has_cloud_attributes() was removed because GetFileAttributesW can BLOCK
            // indefinitely on cloud-only OneDrive folders
            let is_onedrive_base = onedrive::is_onedrive_path(&base_path_buf);
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
                &directory_cache,
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
                &mut batch_size,
                &mut batch_tracker,
                &mut batch_start,
                &mut batch,
                &mut all_entries_disk,
                &file_entry_sender,
                &ctx,
                &disk_cache,
                &directory_cache,
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
                &ctx,
                &disk_cache,
                &directory_cache,
                &directory_index_opt,
                show_hidden,
            );
            // DISABLED: Direct subdirectory prefetch (testing HDD I/O impact)
            // if !is_ssd && gen_clone.load(AtomicOrdering::Relaxed) == my_gen {
            //     let subdirs: Vec<PathBuf> = all_entries_disk
            //         .iter()
            //         .filter(|e| e.is_dir)
            //         .take(5)
            //         .map(|e| e.path.clone())
            //         .collect();
            //     if !subdirs.is_empty() {
            //         let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
            //     }
            // }
        });
    }
}

