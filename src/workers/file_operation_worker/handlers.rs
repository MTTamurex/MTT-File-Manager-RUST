use super::{sanitize_operation_path, sanitize_operation_paths, FileOperationResult, SendHwnd};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::infrastructure::windows::recycle_bin;
use crate::infrastructure::windows::shell_operations;

pub(super) fn handle_delete(
    paths: Vec<PathBuf>,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    match sanitize_operation_paths(&paths) {
        Ok(valid_paths) => {
            if valid_paths.is_empty() {
                return;
            }

            // L-4: log when shell delete returns false (silent failure)
            if !shell_operations::delete_items_with_shell(&valid_paths, hwnd.0) {
                log::warn!("[FileOps] delete_items_with_shell returned false for {} paths", valid_paths.len());
            }
            let mut parents = HashSet::new();
            for path in &valid_paths {
                if let Some(parent) = path.parent() {
                    parents.insert(parent.to_path_buf());
                }
            }
            if !parents.is_empty() {
                let _ = result_sender.send(FileOperationResult::DeleteCompleted {
                    parent_folders: parents.into_iter().collect(),
                    deleted_paths: valid_paths.clone(),
                });
            }
            let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
        }
        Err(err) => {
            log::warn!("[SECURITY] Delete blocked: {}", err);
        }
    }
}

fn is_invalid_rename_target(new_name: &str) -> bool {
    let invalid_chars = new_name.contains('\0')
        || new_name.contains('\\')
        || new_name.contains('/')
        || new_name.contains('<')
        || new_name.contains('>')
        || new_name.contains(':')
        || new_name.contains('"')
        || new_name.contains('|')
        || new_name.contains('?')
        || new_name.contains('*');
    let base_name = new_name.split('.').next().unwrap_or("");

    invalid_chars
        || new_name == "."
        || new_name == ".."
        || new_name.ends_with('.')
        || new_name.ends_with(' ')
        || crate::infrastructure::security::is_windows_reserved_name(base_name)
}

pub(super) fn handle_rename(
    path: PathBuf,
    new_name: String,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    match sanitize_operation_path(&path) {
        Ok(valid_path) => {
            if crate::infrastructure::windows::is_drive_root_path(&valid_path) {
                match crate::infrastructure::windows::rename_volume_label(&valid_path, &new_name, hwnd.0) {
                    Ok(_) => {
                        let _ = result_sender.send(FileOperationResult::DriveRenameCompleted {
                            drive_path: valid_path,
                            new_label: new_name,
                        });
                    }
                    Err(err) => {
                        let _ = result_sender.send(FileOperationResult::DriveRenameFailed {
                            drive_path: valid_path,
                            error: err.to_string(),
                            cancelled: matches!(err, crate::infrastructure::windows::VolumeLabelRenameError::Cancelled),
                        });
                    }
                }
                return;
            }

            if is_invalid_rename_target(&new_name) {
                log::warn!(
                    "[SECURITY] Rename blocked: invalid target name '{}'",
                    new_name
                );
                return;
            }

            let success = shell_operations::rename_item_with_shell(&valid_path, &new_name, hwnd.0);
            if success {
                if let Some(parent) = valid_path.parent().map(|p| p.to_path_buf()) {
                    let _ = result_sender.send(FileOperationResult::RenameCompleted {
                        path: valid_path,
                        new_name: new_name.clone(),
                        parent_folder: parent,
                    });
                }
            }
        }
        Err(err) => {
            log::warn!("[SECURITY] Rename blocked: {}", err);
        }
    }
}

pub(super) fn handle_copy(
    path: PathBuf,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    let valid_path = sanitize_operation_path(&path);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_path, valid_dest) {
        (Ok(path), Ok(dest_folder)) => {
            if crate::infrastructure::windows::is_shell_navigation_path(&path, false) {
                let _ = shell_operations::copy_item_with_file_op(&path, &dest_folder, hwnd.0);
            } else {
                let _ = shell_operations::copy_item_with_shell(&path, &dest_folder, hwnd.0);
            }
            let _ = result_sender.send(FileOperationResult::CopyCompleted { dest_folder });
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Copy blocked: {}", err);
        }
    }
}

pub(super) fn handle_move(
    path: PathBuf,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    let valid_path = sanitize_operation_path(&path);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_path, valid_dest) {
        (Ok(path), Ok(dest_folder)) => {
            // Capture source folder before move
            let source_folder = path.parent().map(|p| p.to_path_buf());
            // Use IFileOperation for virtual paths (like items inside archives)
            let success = if crate::infrastructure::windows::is_shell_navigation_path(&path, false)
            {
                shell_operations::move_item_with_file_op(&path, &dest_folder, hwnd.0)
            } else {
                shell_operations::move_item_with_shell(&path, &dest_folder, hwnd.0)
            };

            if success {
                if let Some(src) = source_folder {
                    let _ = result_sender.send(FileOperationResult::MoveCompleted {
                        source_folder: src,
                        dest_folder,
                    });
                }
            }
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Move blocked: {}", err);
        }
    }
}

pub(super) fn handle_copy_batch(
    paths: Vec<PathBuf>,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    let valid_paths = sanitize_operation_paths(&paths);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_paths, valid_dest) {
        (Ok(paths), Ok(dest_folder)) => {
            let has_virtual_path = paths
                .iter()
                .any(|p| crate::infrastructure::windows::is_shell_navigation_path(p, false));

            let success = if has_virtual_path {
                shell_operations::copy_items_with_file_op(&paths, &dest_folder, hwnd.0)
            } else {
                shell_operations::copy_items_with_shell(&paths, &dest_folder, hwnd.0)
            };

            if success {
                let _ = result_sender.send(FileOperationResult::CopyCompleted { dest_folder });
            }
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Copy batch blocked: {}", err);
        }
    }
}

pub(super) fn handle_move_batch(
    paths: Vec<PathBuf>,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    let valid_paths = sanitize_operation_paths(&paths);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_paths, valid_dest) {
        (Ok(paths), Ok(dest_folder)) => {
            // Collect unique source folders before move
            let mut source_folders = HashSet::new();
            for path in &paths {
                if let Some(parent) = path.parent() {
                    source_folders.insert(parent.to_path_buf());
                }
            }

            let has_virtual_path = paths
                .iter()
                .any(|p| crate::infrastructure::windows::is_shell_navigation_path(p, false));

            let success = if has_virtual_path {
                shell_operations::move_items_with_file_op(&paths, &dest_folder, hwnd.0)
            } else {
                shell_operations::move_items_with_shell(&paths, &dest_folder, hwnd.0)
            };

            if success && !source_folders.is_empty() {
                let _ = result_sender.send(FileOperationResult::MoveBatchCompleted {
                    source_folders: source_folders.into_iter().collect(),
                    dest_folder,
                    moved_files: paths,
                });
            }
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Move batch blocked: {}", err);
        }
    }
}

pub(super) fn handle_restore_from_recycle_bin(
    items: Vec<(PathBuf, PathBuf)>,
    result_sender: &Sender<FileOperationResult>,
) {
    let mut parents = HashSet::new();
    for (physical_path, original_path) in items {
        let valid_physical = sanitize_operation_path(&physical_path);
        let valid_original = sanitize_operation_path(&original_path);
        match (valid_physical, valid_original) {
            (Ok(physical_path), Ok(original_path)) => {
                if let Some(parent) = original_path.parent() {
                    parents.insert(parent.to_path_buf());
                }
                let _ = recycle_bin::restore_from_recycle_bin(&physical_path, &original_path);
            }
            (Err(err), _) | (_, Err(err)) => {
                log::warn!("[SECURITY] Restore blocked: {}", err);
            }
        }
    }
    if !parents.is_empty() {
        let _ = result_sender.send(FileOperationResult::RestoreCompleted {
            parent_folders: parents.into_iter().collect(),
        });
    }
    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
}

pub(super) fn handle_delete_permanently(
    physical_paths: Vec<PathBuf>,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    match sanitize_operation_paths(&physical_paths) {
        Ok(valid_paths) => {
            if valid_paths.is_empty() {
                return;
            }
            let success =
                shell_operations::delete_items_permanently_with_shell(&valid_paths, hwnd.0);
            if !success {
                log::warn!("[FILE-OP] Permanent delete cancelled or failed");
                return;
            }
            let mut parents = std::collections::HashSet::new();
            for path in &valid_paths {
                if let Some(parent) = path.parent() {
                    parents.insert(parent.to_path_buf());
                }
            }
            if !parents.is_empty() {
                let _ = result_sender.send(FileOperationResult::DeleteCompleted {
                    parent_folders: parents.into_iter().collect(),
                    deleted_paths: valid_paths.clone(),
                });
            }
            let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
        }
        Err(err) => {
            log::warn!("[SECURITY] Permanent delete blocked: {}", err);
        }
    }
}

pub(super) fn handle_empty_recycle_bin(
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    let _ = recycle_bin::empty_recycle_bin(hwnd.0);
    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
}

/// Shows Windows Properties dialog for each path in the background.
/// Fire-and-forget: `SHObjectProperties` opens a modeless dialog that manages its own lifetime.
pub(super) fn handle_show_properties(paths: Vec<std::path::PathBuf>, hwnd: SendHwnd) {
    use crate::infrastructure::windows::native_menu::show_properties_dialog;
    for path in &paths {
        if let Err(e) = show_properties_dialog(hwnd.0, path) {
            log::warn!("[PROPERTIES] Failed to show properties for {}: {:?}", path.display(), e);
        }
    }
}
