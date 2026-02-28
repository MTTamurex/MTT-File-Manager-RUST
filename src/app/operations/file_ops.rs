//! File operations: delete, create folder, rename, properties, shortcuts
//!
//! This module handles basic file operations interacting with the shell.

use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::security::classify_shell_namespace_path;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SHELL_OPEN_CONFIRMATION_WINDOW: Duration = Duration::from_secs(10);

fn is_explicit_shell_namespace_path(path: &Path) -> bool {
    classify_shell_namespace_path(path).is_some()
}

fn is_unc_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if !s.starts_with(r"\\") {
        return false;
    }

    match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.starts_with("UNC\\"),
        None => true,
    }
}

fn is_high_risk_shell_open_source(path: &Path) -> bool {
    is_unc_path(path) || is_explicit_shell_namespace_path(path)
}

fn is_onedrive_media_file(path: &Path) -> bool {
    let is_cloud = crate::infrastructure::onedrive::is_onedrive_path(path)
        || crate::infrastructure::onedrive::path_has_cloud_attributes(path);

    if !is_cloud {
        return false;
    }

    path.extension()
        .and_then(|e| e.to_str())
        .map(crate::infrastructure::windows::is_media_extension)
        .unwrap_or(false)
}

impl ImageViewerApp {
    pub fn open_with_shell_guarded(&mut self, path: &Path) {
        if is_high_risk_shell_open_source(path) {
            let now = Instant::now();
            let confirmed = self
                .pending_shell_open_confirmation
                .as_ref()
                .map(|(pending_path, pending_at)| {
                    pending_path == path
                        && now.duration_since(*pending_at) <= SHELL_OPEN_CONFIRMATION_WINDOW
                })
                .unwrap_or(false);

            if !confirmed {
                self.pending_shell_open_confirmation = Some((path.to_path_buf(), now));
                self.notifications.push(crate::application::AppNotification::warning(
                    "Fonte de alto risco (UNC/Shell). Clique novamente para confirmar a abertura."
                        .to_string(),
                ));
                return;
            }

            self.pending_shell_open_confirmation = None;
        } else {
            self.pending_shell_open_confirmation = None;
        }

        if let Err(e) = file_operations::open_with_shell(path, self.native_hwnd) {
            log::warn!(
                "[SECURITY] Shell open failed for '{}': {}",
                path.display(),
                e
            );
            self.notifications.push(crate::application::AppNotification::warning(
                "Falha ao abrir item com o aplicativo padrão.".to_string(),
            ));
        } else if is_onedrive_media_file(path) {
            // Hydration/open may not always emit a watcher path that triggers thumbnail retry.
            // Force a light retry path for media files in OneDrive.
            let path_buf = path.to_path_buf();
            crate::workers::thumbnail::clear_failure_cache(&path_buf);
            self.cache_manager.failed_thumbnails.pop(&path_buf);

            // Requeue thumbnail extraction while keeping current visual until new data arrives.
            self.request_thumbnail_load_with_modified(path_buf, self.thumbnail_size as u32, 0);
        }
    }

    pub fn delete_with_shell_for_idx(&mut self, idx: Option<usize>) {
        let paths = self.context_target_paths(idx);
        self.delete_with_shell_for_paths(&paths);
    }

    pub fn delete_permanently_for_idx(&mut self, idx: Option<usize>) {
        let paths = self.context_target_paths(idx);
        if paths.is_empty() {
            return;
        }

        self.file_operation_state.file_ops_in_progress += 1;
        if self.file_operation_state.file_op_sender.send(
            crate::workers::file_operation_worker::FileOperationRequest::delete_permanently(
                paths.clone(),
                self.native_hwnd.unwrap_or_default(),
            ),
        ).is_err() {
            self.file_operation_state.file_ops_in_progress =
                self.file_operation_state.file_ops_in_progress.saturating_sub(1);
            log::warn!("[FileOps] H-3: worker channel closed on delete_permanently");
        }

        for path in &paths {
            self.file_operation_state
                .pending_deletions
                .insert(path.clone(), ());
        }
        self.thumbnail_queue.remove_paths(&paths);

        for path in &paths {
            self.disk_cache.remove_cache_for_path(path);
            self.multi_selection.remove(path);
        }

        if let Some(selected) = &self.selected_file {
            if paths.contains(&selected.path) {
                self.selected_item = None;
                self.selected_file = None;
            }
        }
    }

    pub fn delete_with_shell_for_paths(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }

        // Send request to background worker (BATCH)
        self.file_operation_state.file_ops_in_progress += 1;
        if self.file_operation_state.file_op_sender.send(
            crate::workers::file_operation_worker::FileOperationRequest::delete(
                paths.to_vec(),
                self.native_hwnd.unwrap_or_default(),
            ),
        ).is_err() {
            self.file_operation_state.file_ops_in_progress =
                self.file_operation_state.file_ops_in_progress.saturating_sub(1);
            log::warn!("[FileOps] H-3: worker channel closed on delete");
        }

        // Track pending deletions to suppress thumbnail extraction for these files
        for path in paths {
            self.file_operation_state
                .pending_deletions
                .insert(path.clone(), ());
        }
        self.thumbnail_queue.remove_paths(paths);

        for path in paths {
            // Clear cache and selection proactively
            self.disk_cache.remove_cache_for_path(path);
            self.multi_selection.remove(path);
        }

        // Reset primary selection if it was deleted
        if let Some(selected) = &self.selected_file {
            if paths.contains(&selected.path) {
                self.selected_item = None;
                self.selected_file = None;
            }
        }
    }

    pub fn show_properties_for_idx(&mut self, idx: Option<usize>) {
        let paths = self.context_target_paths(idx);
        if paths.is_empty() {
            return;
        }

        let Some(hwnd) = self.native_hwnd else {
            return;
        };

        // Dispatch to the file operation worker (STA COM thread) — avoids blocking the UI thread.
        // SHObjectProperties opens a modeless dialog that manages its own lifetime.
        let _ = self
            .file_operation_state
            .file_op_sender
            .send(crate::workers::file_operation_worker::FileOperationRequest::show_properties(
                paths, hwnd,
            ));
        // Note: do NOT increment file_ops_in_progress — this is fire-and-forget.
    }

    pub fn create_new_folder(&mut self) {
        if self.navigation_state.is_computer_view || self.navigation_state.is_recycle_bin_view {
            return;
        }

        let base_path = PathBuf::from(&self.navigation_state.current_path);

        match file_operations::create_new_folder(&base_path) {
            Ok(full_path) => {
                let new_folder_name = full_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                // CRITICAL: Immediately create entry to allow renaming
                let new_item = FileEntry::from_path(full_path.clone(), true);

                self.all_items.push(new_item);
                self.filter_items();
                self.sort_items();

                // Find index in filtered vector
                if let Some(idx) = self.items.iter().position(|i| i.path == full_path) {
                    self.selected_item = Some(idx);
                    self.selected_file = Some(self.items[idx].clone());
                    self.renaming_state = Some((idx, new_folder_name));
                    self.focus_rename = true;
                    self.scroll_to_selected = true;
                }

                if let Some(parent) = full_path.parent() {
                    self.directory_cache.invalidate(&parent.to_path_buf());
                    if let Some(directory_index) = &self.directory_index {
                        let _ = directory_index.invalidate(parent);
                    }
                }

                self.ui_ctx.request_repaint();
            }
            Err(e) => {
                log::error!("Erro ao criar pasta: {}", e);
            }
        }
    }

    /// Renomeia arquivo usando Shell API via Background Worker
    pub fn rename_with_shell(&mut self, idx: usize) {
        if let Some((_, new_name)) = self.renaming_state.take() {
            if let Some(item) = self.items.get(idx) {
                // Send request to background worker
                self.file_operation_state.file_ops_in_progress += 1;
                if self.file_operation_state.file_op_sender.send(
                    crate::workers::file_operation_worker::FileOperationRequest::rename(
                        item.path.clone(),
                        new_name,
                        self.native_hwnd.unwrap_or_default(),
                    ),
                ).is_err() {
                    self.file_operation_state.file_ops_in_progress =
                        self.file_operation_state.file_ops_in_progress.saturating_sub(1);
                    log::warn!("[FileOps] H-3: worker channel closed on rename");
                }
            }
        }
    }

    /// Create a Windows shell shortcut (.lnk) pointing to `target` in the same directory.
    pub fn create_shell_shortcut(&self, target: &Path) -> Result<PathBuf, String> {
        file_operations::create_shortcut(target, &self.navigation_state.current_path)
    }

    /// Mounts an ISO programmatically and marks it for auto-navigation
    pub fn mount_and_navigate_iso(&mut self, path: PathBuf) {
        use crate::infrastructure::windows::mount_iso;

        self.file_operation_state.pending_iso_mount = Some(path.clone());

        match mount_iso(&path) {
            Ok(_) => {
                // Notify the start of the mount operation
                self.notifications
                    .push(crate::application::AppNotification::info(format!(
                        "Montando ISO: {}",
                        path.file_name()
                            .map(|n| n.to_string_lossy())
                            .unwrap_or_default()
                    )));
            }
            Err(e) => {
                self.file_operation_state.pending_iso_mount = None;
                self.notifications
                    .push(crate::application::AppNotification::error(format!(
                        "Falha ao montar ISO: {}",
                        e
                    )));
            }
        }
    }
}
