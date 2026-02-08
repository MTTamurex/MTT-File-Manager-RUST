use crate::app::state::ImageViewerApp;
use crate::workers::file_operation_worker::FileOperationResult;
use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;

impl ImageViewerApp {
    pub(super) fn process_file_operation_results(&mut self, current_path_norm: &str) {
        loop {
            match self.file_op_res_receiver.try_recv() {
                Ok(res) => match res {
                    FileOperationResult::RenameCompleted {
                        path,
                        new_name,
                        parent_folder,
                    } => self.handle_rename_completed(
                        path,
                        new_name,
                        parent_folder,
                        current_path_norm,
                    ),
                    FileOperationResult::RecycleBinChanged => self.handle_recycle_bin_changed(),
                    FileOperationResult::RestoreCompleted { parent_folders } => {
                        self.handle_parent_folder_updates(parent_folders, current_path_norm)
                    }
                    FileOperationResult::DeleteCompleted { parent_folders } => {
                        self.handle_parent_folder_updates(parent_folders, current_path_norm)
                    }
                    FileOperationResult::CopyCompleted { dest_folder } => {
                        self.handle_copy_completed(dest_folder, current_path_norm)
                    }
                    FileOperationResult::MoveCompleted {
                        source_folder,
                        dest_folder,
                    } => self.handle_move_completed(source_folder, dest_folder, current_path_norm),
                    FileOperationResult::MoveBatchCompleted {
                        source_folders,
                        dest_folder,
                        moved_files,
                    } => self.handle_move_batch_completed(
                        source_folders,
                        dest_folder,
                        moved_files,
                        current_path_norm,
                    ),
                    FileOperationResult::Finished => self.handle_file_operation_finished(),
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }

    fn invalidate_folder_and_tab_caches(&mut self, folder: &Path) {
        self.invalidate_directory_caches(folder);
        let folder_norm = Self::normalize_for_match(folder);
        self.clear_tab_cache_for_normalized_path(&folder_norm);
    }

    fn handle_rename_completed(
        &mut self,
        path: PathBuf,
        new_name: String,
        parent_folder: PathBuf,
        current_path_norm: &str,
    ) {
        let parent_str = Self::normalize_for_match(parent_folder.as_path());
        let path_str = Self::normalize_for_match(&path);
        self.invalidate_folder_and_tab_caches(&parent_folder);

        // Keep details panel selection in sync before reload completes.
        if let Some(selected) = &mut self.selected_file {
            if Self::normalize_for_match(&selected.path) == path_str {
                let new_path = parent_folder.join(&new_name);
                selected.path = new_path;
                selected.name = new_name.clone();
            }
        }

        // If renamed file is currently playing, drop stale preview state.
        let should_destroy_preview = match self.media_preview.as_ref() {
            Some(crate::ui::components::media_preview::MediaPreview::Video(player)) => {
                Self::normalize_for_match(&player.path) == path_str
            }
            _ => false,
        };
        if should_destroy_preview {
            self.destroy_media_preview();
        }

        if parent_str == current_path_norm {
            let new_path = parent_folder.join(&new_name);
            self.pending_select_path = Some(new_path);
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

    fn handle_recycle_bin_changed(&mut self) {
        if self.is_recycle_bin_view {
            #[cfg(debug_assertions)]
            eprintln!("[RECYCLE] Operation finished, refreshing view.");
            self.setup_recycle_bin_view();
            // Keep tab manager synchronized with recycle-bin virtual view.
            self.sync_to_tab();
        }
    }

    fn handle_parent_folder_updates(
        &mut self,
        parent_folders: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        let mut should_reload_current = false;
        for parent in parent_folders {
            self.invalidate_folder_and_tab_caches(&parent);
            let parent_str = Self::normalize_for_match(parent.as_path());
            if parent_str == current_path_norm {
                should_reload_current = true;
            }
        }

        if should_reload_current {
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

    fn handle_copy_completed(&mut self, dest_folder: PathBuf, current_path_norm: &str) {
        let dest_str = Self::normalize_for_match(dest_folder.as_path());
        self.invalidate_folder_and_tab_caches(&dest_folder);

        // Retry files that previously failed thumbnail extraction while copy was in progress.
        self.cache_manager.clear_failed();
        crate::workers::thumbnail::clear_all_failures();

        if dest_str == current_path_norm {
            #[cfg(debug_assertions)]
            eprintln!(
                "[COPY] Dest folder matches current view, reloading: {}",
                self.current_path
            );
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

    fn handle_move_completed(
        &mut self,
        source_folder: PathBuf,
        dest_folder: PathBuf,
        current_path_norm: &str,
    ) {
        let source_str = Self::normalize_for_match(source_folder.as_path());
        let dest_str = Self::normalize_for_match(dest_folder.as_path());

        self.invalidate_directory_caches(&source_folder);
        self.invalidate_directory_caches(&dest_folder);
        self.clear_tab_cache_for_normalized_path(&source_str);
        self.clear_tab_cache_for_normalized_path(&dest_str);

        self.cache_manager.clear_failed();
        crate::workers::thumbnail::clear_all_failures();

        if current_path_norm == source_str {
            #[cfg(debug_assertions)]
            eprintln!(
                "[MOVE] Source folder matches current view, reloading: {}",
                self.current_path
            );
            self.loaded_path.clear();
            self.load_folder(false);
        }

        if current_path_norm == dest_str {
            #[cfg(debug_assertions)]
            eprintln!(
                "[MOVE] Dest folder matches current view, reloading: {}",
                self.current_path
            );
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

    fn handle_move_batch_completed(
        &mut self,
        source_folders: Vec<PathBuf>,
        dest_folder: PathBuf,
        moved_files: Vec<PathBuf>,
        current_path_norm: &str,
    ) {
        let dest_str = Self::normalize_for_match(dest_folder.as_path());

        self.cache_manager.clear_failed();
        crate::workers::thumbnail::clear_all_failures();

        for source_folder in &source_folders {
            self.invalidate_directory_caches(source_folder);
        }
        self.invalidate_directory_caches(&dest_folder);

        // If any moved file was a folder cover, force re-discovery.
        for source_folder in &source_folders {
            let covers = self
                .disk_cache
                .get_folder_covers(std::slice::from_ref(source_folder));
            if let Some(current_cover) = covers.get(source_folder) {
                if moved_files.iter().any(|f| f == current_cover) {
                    self.disk_cache.remove_folder_cover(source_folder);
                    self.cache_manager.folder_preview_cache.pop(source_folder);
                    let _ = self.cover_worker_sender.send(source_folder.clone());
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[MOVE-BATCH] Moved file was folder cover for {:?}, requesting recalculation",
                        source_folder
                    );
                }
            }
        }

        let mut should_reload_source_view = false;
        for source_folder in &source_folders {
            let source_str = Self::normalize_for_match(source_folder.as_path());
            if current_path_norm == source_str {
                should_reload_source_view = true;
            }
            self.clear_tab_cache_for_normalized_path(&source_str);
        }
        self.clear_tab_cache_for_normalized_path(&dest_str);

        if should_reload_source_view {
            self.loaded_path.clear();
            self.load_folder(false);
        }

        if current_path_norm == dest_str {
            #[cfg(debug_assertions)]
            eprintln!(
                "[MOVE-BATCH] Dest folder matches current view, reloading: {}",
                self.current_path
            );
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

    fn handle_file_operation_finished(&mut self) {
        self.file_ops_in_progress = self.file_ops_in_progress.saturating_sub(1);
        if self.file_ops_in_progress == 0 {
            // Completion handlers already triggered reloads. Skip watcher queued reload.
            self.pending_auto_reload = false;
            // Keep pending_deletions until folder load completion to avoid stale thumbnail retries.
            if !self.is_loading_folder {
                self.pending_deletions.clear();
            }
        }
    }
}
