//! Clipboard operations: copy, cut, paste, copy path
//!
//! This module handles interaction with the Windows clipboard for file operations.

use crate::app::state::ImageViewerApp;
use crate::application::file_operations;
use std::path::{Path, PathBuf};

fn normalize_path_for_hierarchy(path: &Path) -> String {
    let lower = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let normalized = if let Some(stripped) = lower.strip_prefix(r"\\?\unc\") {
        format!(r"\\{stripped}")
    } else if let Some(stripped) = lower.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        lower
    };
    normalized.trim_end_matches('\\').to_string()
}

fn path_is_same_or_ancestor(ancestor: &Path, descendant: &Path) -> bool {
    let ancestor = normalize_path_for_hierarchy(ancestor);
    let descendant = normalize_path_for_hierarchy(descendant);
    descendant == ancestor || descendant.starts_with(&format!(r"{ancestor}\"))
}

fn paste_target_is_valid_for_sources(sources: &[PathBuf], target: &Path) -> bool {
    !sources
        .iter()
        .any(|source| path_is_same_or_ancestor(source, target))
}

fn context_paste_destination(
    target_paths: &[PathBuf],
    operation_directory: Option<&Path>,
    primary_is_directory: Option<bool>,
) -> Option<PathBuf> {
    if primary_is_directory == Some(true) {
        target_paths.first().cloned()
    } else {
        operation_directory.map(Path::to_path_buf)
    }
}

impl ImageViewerApp {
    pub(crate) fn path_is_same_or_ancestor_of_open_panel(&self, path: &Path) -> bool {
        path_is_same_or_ancestor(path, Path::new(&self.navigation_state.current_path))
            || self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|snapshot| path_is_same_or_ancestor(path, Path::new(&snapshot.path)))
    }

    pub(crate) fn path_is_archive_namespace(path: &Path) -> bool {
        if let Some((archive_path, _)) = crate::domain::file_entry::split_archive_path(path) {
            return std::fs::metadata(&archive_path)
                .map(|metadata| metadata.is_file())
                .unwrap_or(true);
        }

        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(crate::domain::file_entry::is_archive_extension)
            && std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
    }

    pub(crate) fn current_location_is_archive_namespace(&self) -> bool {
        Self::path_is_archive_namespace(Path::new(&self.navigation_state.current_path))
    }

    pub(crate) fn context_target_is_directory(&self, idx: Option<usize>, path: &Path) -> bool {
        if self
            .context_menu
            .target_paths
            .first()
            .is_some_and(|target| target == path)
        {
            if let Some(is_directory) = self.context_menu.primary_is_directory {
                return is_directory;
            }
        }

        if crate::infrastructure::windows::is_drive_root_path(path) {
            return true;
        }

        if let Some(index) = idx {
            if let Some(item) = self.items.get(index) {
                if item.path == path {
                    return item.is_dir || item.drive_info.is_some();
                }
            }
        }

        if let Some(selected) = self.selected_file.as_ref() {
            if selected.path == path {
                return selected.is_dir || selected.drive_info.is_some();
            }
        }

        if let Some(item) = self.items.iter().find(|item| item.path == path) {
            return item.is_dir || item.drive_info.is_some();
        }

        if self
            .pinned_folders
            .iter()
            .any(|pinned| Path::new(&pinned.path) == path)
        {
            return true;
        }

        path == Path::new(&self.navigation_state.current_path)
    }

    pub fn can_copy_from_current_location(&self) -> bool {
        !self.navigation_state.is_recycle_bin_view
    }

    pub fn can_paste_into_current_location(&self) -> bool {
        !self.navigation_state.is_computer_view
            && !self.navigation_state.is_recycle_bin_view
            && self.can_paste_into_path(Path::new(&self.navigation_state.current_path))
    }

    pub(crate) fn can_paste_into_path(&self, path: &Path) -> bool {
        self.clipboard.has_content()
            && !path.as_os_str().is_empty()
            && !path
                .to_str()
                .is_some_and(crate::domain::special_paths::is_virtual_path)
            && !Self::path_is_archive_namespace(path)
    }

    pub fn command_copy(&mut self, idx: Option<usize>) {
        if !self.can_copy_from_current_location() {
            self.context_menu.target_paths.clear();
            return;
        }

        if idx.is_none() && !self.context_menu.target_paths.is_empty() {
            let owner = self.shell_op_hwnd();
            self.clipboard
                .copy(&self.context_menu.target_paths.clone(), owner);
            return;
        }

        let mut files = Vec::new();

        let use_multi_selection = if let Some(i) = idx {
            if let Some(item) = self.items.get(i) {
                self.multi_selection.contains(&item.path)
            } else {
                false
            }
        } else {
            !self.multi_selection.is_empty()
        };

        if use_multi_selection {
            files.extend(self.multi_selection.iter().cloned());
        } else if let Some(i) = idx.or(self.selected_item) {
            if let Some(item) = self.items.get(i) {
                files.push(item.path.clone());
            }
        }

        if !files.is_empty() {
            let owner = self.shell_op_hwnd();
            self.clipboard.copy(&files, owner);
        }
    }

    pub(crate) fn copy_paths_to_clipboard(&mut self, paths: &[PathBuf]) {
        if paths.is_empty() {
            return;
        }
        let owner = self.shell_op_hwnd();
        self.clipboard.copy(paths, owner);
    }

    /// Cut: place the file on the Windows clipboard with the MOVE flag
    pub fn command_cut(&mut self, idx: Option<usize>) {
        if idx.is_none() && !self.context_menu.target_paths.is_empty() {
            if self.context_menu.target_paths.iter().any(|path| {
                crate::domain::file_entry::is_path_inside_existing_archive_file(path)
                    || self.path_is_same_or_ancestor_of_open_panel(path)
            }) {
                self.context_menu.target_paths.clear();
                return;
            }
            let owner = self.shell_op_hwnd();
            self.clipboard
                .cut(&self.context_menu.target_paths.clone(), owner);
            return;
        }

        let mut files = Vec::new();

        let use_multi_selection = if let Some(i) = idx {
            if let Some(item) = self.items.get(i) {
                self.multi_selection.contains(&item.path)
            } else {
                false
            }
        } else {
            !self.multi_selection.is_empty()
        };

        if use_multi_selection {
            files.extend(self.multi_selection.iter().cloned());
        } else if let Some(i) = idx.or(self.selected_item) {
            if let Some(item) = self.items.get(i) {
                files.push(item.path.clone());
            }
        }

        if !files.is_empty()
            && !files.iter().any(|path| {
                crate::domain::file_entry::is_path_inside_existing_archive_file(path)
                    || self.path_is_same_or_ancestor_of_open_panel(path)
            })
        {
            let owner = self.shell_op_hwnd();
            self.clipboard.cut(&files, owner);
        }
    }

    pub(crate) fn cut_paths_to_clipboard(&mut self, paths: &[PathBuf]) {
        if paths.is_empty()
            || paths.iter().any(|path| {
                crate::domain::file_entry::is_path_inside_existing_archive_file(path)
                    || self.path_is_same_or_ancestor_of_open_panel(path)
            })
        {
            return;
        }
        let owner = self.shell_op_hwnd();
        self.clipboard.cut(paths, owner);
    }

    /// Paste: Reads from clipboard using ClipboardManager via Background Worker
    pub fn command_paste(&mut self, idx: Option<usize>) {
        log::debug!(
            "[PASTE-DIAG] command_paste called with idx: {:?}, ops_in_progress: {}",
            idx,
            self.file_operation_state.file_ops_in_progress
        );

        let has_explicit_context_target =
            idx.is_none() && !self.context_menu.target_paths.is_empty();
        if !has_explicit_context_target
            && (self.navigation_state.is_computer_view || self.navigation_state.is_recycle_bin_view)
        {
            self.context_menu.target_paths.clear();
            return;
        }

        // Destination folder
        let dest_folder = if has_explicit_context_target {
            context_paste_destination(
                &self.context_menu.target_paths,
                self.context_menu.operation_directory.as_deref(),
                self.context_menu.primary_is_directory,
            )
        } else if let Some(idx) = idx {
            Some(if let Some(item) = self.items.get(idx) {
                if item.is_dir {
                    item.path.clone()
                } else {
                    PathBuf::from(&self.navigation_state.current_path)
                }
            } else {
                PathBuf::from(&self.navigation_state.current_path)
            })
        } else {
            Some(PathBuf::from(&self.navigation_state.current_path))
        };
        let Some(dest_folder) = dest_folder else {
            self.context_menu.target_paths.clear();
            return;
        };

        if !self.can_paste_into_path(&dest_folder) {
            self.context_menu.target_paths.clear();
            return;
        }

        // 1. Get files and operation from clipboard via Manager
        // Optimized to use the manager's logic which checks system then internal.
        if let Some((files_to_op, is_move, clipboard_sequence)) =
            self.clipboard.get_files_to_paste()
        {
            if !paste_target_is_valid_for_sources(&files_to_op, &dest_folder) {
                self.context_menu.target_paths.clear();
                return;
            }
            let hwnd = self.shell_op_hwnd();

            if let Some(cleanup_sequence) =
                self.file_operation_state.pending_clipboard_cleanup_sequence
            {
                if clipboard_sequence == Some(cleanup_sequence) {
                    if self.clipboard.clear_completed_move(cleanup_sequence) {
                        self.file_operation_state.pending_clipboard_cleanup_sequence = None;
                    }
                    self.context_menu.target_paths.clear();
                    return;
                }
                self.file_operation_state.pending_clipboard_cleanup_sequence = None;
            }

            if self
                .file_operation_state
                .pending_clipboard_move
                .is_some_and(|pending| pending.clipboard_sequence != clipboard_sequence)
            {
                self.file_operation_state.pending_clipboard_move = None;
            }
            if is_move && self.file_operation_state.pending_clipboard_move.is_some() {
                log::warn!("[FileOps] Ignoring a second clipboard move while one is pending");
                self.context_menu.target_paths.clear();
                return;
            }

            // 2. Dispatch as a single batch operation (single Windows progress dialog)
            let virtual_archive_count = files_to_op
                .iter()
                .filter(|path| {
                    crate::domain::file_entry::is_path_inside_existing_archive_file(path)
                })
                .count();
            if is_move && virtual_archive_count > 0 && virtual_archive_count < files_to_op.len() {
                log::warn!(
                    "[FileOps] Refusing mixed clipboard move with archive and filesystem paths"
                );
                self.context_menu.target_paths.clear();
                return;
            }
            let pending_clipboard_move = if is_move && virtual_archive_count == 0 {
                let Some(clipboard_sequence) = clipboard_sequence else {
                    log::warn!(
                        "[FileOps] Clipboard sequence unavailable; completion cannot safely clear it"
                    );
                    let request =
                        crate::workers::file_operation_worker::FileOperationRequest::move_batch(
                            files_to_op,
                            dest_folder,
                            hwnd,
                        );
                    self.file_operation_state.file_ops_in_progress += 1;
                    if self
                        .file_operation_state
                        .file_op_sender
                        .send(request)
                        .is_err()
                    {
                        self.file_operation_state.file_ops_in_progress = self
                            .file_operation_state
                            .file_ops_in_progress
                            .saturating_sub(1);
                    }
                    self.context_menu.target_paths.clear();
                    return;
                };
                let token = self.file_operation_state.allocate_clipboard_move_token();
                Some(crate::app::file_operation_state::PendingClipboardMove {
                    token,
                    clipboard_sequence: Some(clipboard_sequence),
                })
            } else {
                None
            };
            let req = if let Some(pending) = pending_clipboard_move {
                crate::workers::file_operation_worker::FileOperationRequest::clipboard_move_batch(
                    files_to_op,
                    dest_folder,
                    hwnd,
                    pending.token,
                )
            } else if is_move {
                crate::workers::file_operation_worker::FileOperationRequest::move_batch(
                    files_to_op,
                    dest_folder,
                    hwnd,
                )
            } else {
                crate::workers::file_operation_worker::FileOperationRequest::copy_batch(
                    files_to_op,
                    dest_folder,
                    hwnd,
                )
            };
            self.file_operation_state.file_ops_in_progress += 1;
            let send_succeeded = self.file_operation_state.file_op_sender.send(req).is_ok();
            if !send_succeeded {
                self.file_operation_state.file_ops_in_progress = self
                    .file_operation_state
                    .file_ops_in_progress
                    .saturating_sub(1);
                log::warn!("[FileOps] H-3: worker channel closed on clipboard op");
            }
            if let Some(pending) = pending_clipboard_move {
                self.file_operation_state.pending_clipboard_move =
                    crate::app::file_operation_state::pending_clipboard_move_after_dispatch(
                        self.file_operation_state.pending_clipboard_move,
                        pending,
                        send_succeeded,
                    );
            }
        }

        self.context_menu.target_paths.clear();
    }

    pub fn retry_completed_clipboard_cleanup(&mut self) {
        let Some(sequence) = self.file_operation_state.pending_clipboard_cleanup_sequence else {
            return;
        };
        if self.clipboard.current_sequence() != Some(sequence)
            || self.clipboard.clear_completed_move(sequence)
        {
            self.file_operation_state.pending_clipboard_cleanup_sequence = None;
        }
    }

    pub fn copy_path_to_clipboard(&self, path: &Path) {
        if let Err(e) = file_operations::copy_path_to_clipboard(path) {
            log::error!("Erro clipboard: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        context_paste_destination, paste_target_is_valid_for_sources, path_is_same_or_ancestor,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn hierarchy_check_is_case_insensitive_and_separator_aware() {
        assert!(path_is_same_or_ancestor(
            Path::new(r"C:\A"),
            Path::new(r"c:\a\B")
        ));
        assert!(!path_is_same_or_ancestor(
            Path::new(r"C:\A"),
            Path::new(r"C:\Another")
        ));
    }

    #[test]
    fn paste_rejects_a_folder_destination_inside_itself() {
        let sources = vec![PathBuf::from(r"C:\A")];
        assert!(!paste_target_is_valid_for_sources(
            &sources,
            Path::new(r"C:\A\B")
        ));
        assert!(paste_target_is_valid_for_sources(
            &sources,
            Path::new(r"C:\B")
        ));
    }

    #[test]
    fn context_paste_uses_item_or_origin_directory() {
        let folder = PathBuf::from(r"C:\origin\folder");
        let file = PathBuf::from(r"C:\origin\file.txt");
        let origin = PathBuf::from(r"C:\origin");

        assert_eq!(
            context_paste_destination(std::slice::from_ref(&folder), Some(&origin), Some(true)),
            Some(folder)
        );
        assert_eq!(
            context_paste_destination(&[file], Some(&origin), Some(false)),
            Some(origin)
        );
    }
}
