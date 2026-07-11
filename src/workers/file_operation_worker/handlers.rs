use super::{sanitize_operation_path, sanitize_operation_paths, FileOperationResult, SendHwnd};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use crate::domain::file_entry::is_path_inside_existing_archive_file;
use crate::infrastructure::archive_extract;
use crate::infrastructure::windows::recycle_bin;
use crate::infrastructure::windows::shell_operations;
use crate::workers::archive_extraction_worker::ArchiveExtractionRequest;

fn split_virtual_archive_paths(paths: Vec<PathBuf>) -> (Vec<PathBuf>, Vec<PathBuf>) {
    paths
        .into_iter()
        .partition(|p| is_path_inside_existing_archive_file(p))
}

fn path_requires_native_extraction(path: &Path) -> bool {
    is_path_inside_existing_archive_file(path)
}

/// Indicates whether a handler completed synchronously or dispatched to the archive worker.
pub(super) enum HandlerCompletion {
    /// The handler completed (or failed) synchronously; caller should send FinishedNoRefresh.
    CompletedSynchronously,
    /// The handler dispatched the job to the archive extraction worker; caller must NOT send FinishedNoRefresh.
    DispatchedAsync,
}

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

            if !shell_operations::delete_items_with_shell(&valid_paths, hwnd.0) {
                log::warn!(
                    "[FileOps] Delete cancelled or failed for {} paths",
                    valid_paths.len()
                );
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
                return;
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

fn known_exact_new_copy_dests(
    paths: &[PathBuf],
    dest_folder: &Path,
    contains_virtual_path: bool,
) -> Vec<PathBuf> {
    if contains_virtual_path {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut ambiguous = HashSet::new();
    let mut exact_dests = Vec::new();

    for path in paths {
        if !path.is_file() && !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name() else {
            continue;
        };

        let dest = dest_folder.join(name);
        if dest.exists() {
            continue;
        }

        if seen.insert(dest.clone()) {
            exact_dests.push(dest);
        } else {
            ambiguous.insert(dest);
        }
    }

    exact_dests.retain(|dest| !ambiguous.contains(dest));
    exact_dests
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
                match crate::infrastructure::windows::rename_volume_label(
                    &valid_path,
                    &new_name,
                    hwnd.0,
                ) {
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
                            cancelled: matches!(
                                err,
                                crate::infrastructure::windows::VolumeLabelRenameError::Cancelled
                            ),
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
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_invalid_name").to_string(),
                });
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
            } else {
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
            }
        }
        Err(err) => {
            log::warn!("[SECURITY] Rename blocked: {}", err);
        }
    }
}

pub(super) fn handle_rename_batch(
    renames: Vec<(PathBuf, String)>,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
) {
    let total = renames.len();
    let mut successful_count = 0usize;
    let mut invalid_name_failures = 0usize;
    let mut other_failures = 0usize;

    for (index, (path, new_name)) in renames.into_iter().enumerate() {
        let current_name = new_name.clone();
        match sanitize_operation_path(&path) {
            Ok(valid_path) => {
                if crate::infrastructure::windows::is_drive_root_path(&valid_path) {
                    other_failures += 1;
                } else if is_invalid_rename_target(&new_name) {
                    log::warn!(
                        "[SECURITY] Batch rename blocked item: invalid target name '{}'",
                        new_name
                    );
                    invalid_name_failures += 1;
                } else if shell_operations::rename_item_with_shell(&valid_path, &new_name, hwnd.0) {
                    if let Some(parent) = valid_path.parent().map(|p| p.to_path_buf()) {
                        // Send a per-item completion event so the UI can update
                        // each entry incrementally without a full folder reload.
                        let _ = result_sender.send(FileOperationResult::RenameCompleted {
                            path: valid_path,
                            new_name,
                            parent_folder: parent,
                        });
                        successful_count += 1;
                    }
                } else {
                    other_failures += 1;
                }
            }
            Err(err) => {
                log::warn!("[SECURITY] Batch rename blocked item: {}", err);
                other_failures += 1;
            }
        }

        let _ = result_sender.send(FileOperationResult::RenameBatchProgress {
            completed: index + 1,
            total,
            current_name,
        });
    }

    if successful_count > 0 {
        let _ = result_sender.send(FileOperationResult::RenameBatchCompleted {
            count: successful_count,
        });
    }

    if invalid_name_failures > 0 {
        let _ = result_sender.send(FileOperationResult::OperationFailed {
            message: rust_i18n::t!("operations.error_invalid_name").to_string(),
        });
    }

    if other_failures > 0 {
        let _ = result_sender.send(FileOperationResult::OperationFailed {
            message: rust_i18n::t!("operations.error_cancelled").to_string(),
        });
    }
}

pub(super) fn handle_copy(
    path: PathBuf,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
    archive_extract_sender: &Sender<ArchiveExtractionRequest>,
) -> HandlerCompletion {
    let valid_path = sanitize_operation_path(&path);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_path, valid_dest) {
        (Ok(path), Ok(dest_folder)) => {
            let is_virtual = path_requires_native_extraction(&path);
            let native_ok = archive_extract::has_native_support(std::slice::from_ref(&path));
            log::debug!(
                "[FileOps] handle_copy: path={}, is_virtual={}, native_support={}",
                path.display(),
                is_virtual,
                native_ok
            );

            let copied_dests =
                known_exact_new_copy_dests(std::slice::from_ref(&path), &dest_folder, is_virtual);

            if is_virtual && native_ok {
                log::debug!(
                    "[FileOps] Dispatching native archive extraction for: {}",
                    path.display()
                );
                match archive_extract_sender.send(ArchiveExtractionRequest::Copy {
                    paths: vec![path],
                    dest_folder,
                    copied_dests,
                }) {
                    Ok(()) => return HandlerCompletion::DispatchedAsync,
                    Err(e) => {
                        log::warn!("[FileOps] Failed to dispatch archive extraction: {}", e);
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
                        return HandlerCompletion::CompletedSynchronously;
                    }
                }
            }

            // Prefer IFileOperation for all copy/move paths. It matches modern
            // Explorer behavior better than SHFileOperationW, especially for
            // virtual filesystem providers such as Cryptomator.
            let success = shell_operations::copy_item_with_file_op(&path, &dest_folder, hwnd.0);
            log::debug!("[FileOps] handle_copy result: success={}", success);

            if success {
                let _ = result_sender.send(FileOperationResult::CopyCompleted {
                    dest_folder,
                    copied_dests,
                });
            } else {
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
            }
            HandlerCompletion::CompletedSynchronously
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Copy blocked: {}", err);
            HandlerCompletion::CompletedSynchronously
        }
    }
}

pub(super) fn handle_move(
    path: PathBuf,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
    archive_extract_sender: &Sender<ArchiveExtractionRequest>,
) -> HandlerCompletion {
    // Items inside archives cannot be moved by the Shell. Downgrade to a copy.
    if is_path_inside_existing_archive_file(&path) {
        log::debug!(
            "[FileOps] handle_move downgraded to copy (path inside archive): {}",
            path.display()
        );
        return handle_copy(
            path,
            dest_folder,
            hwnd,
            result_sender,
            archive_extract_sender,
        );
    }

    let valid_path = sanitize_operation_path(&path);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_path, valid_dest) {
        (Ok(path), Ok(dest_folder)) => {
            // Capture source folder before move
            let source_folder = path.parent().map(|p| p.to_path_buf());
            // Use IFileOperation for virtual paths (like items inside archives)
            let is_virtual = path_requires_native_extraction(&path);
            let native_ok = archive_extract::has_native_support(std::slice::from_ref(&path));
            log::debug!(
                "[FileOps] handle_move: path={}, is_virtual={}, native_support={}",
                path.display(),
                is_virtual,
                native_ok
            );

            if is_virtual && native_ok {
                log::debug!(
                    "[FileOps] Dispatching native archive extraction (move) for: {}",
                    path.display()
                );
                let Some(source_folder) = source_folder.clone() else {
                    let _ = result_sender.send(FileOperationResult::OperationFailed {
                        message: rust_i18n::t!("operations.error_cancelled").to_string(),
                    });
                    return HandlerCompletion::CompletedSynchronously;
                };
                let moved_dest = known_exact_move_dest(&path, &dest_folder);
                match archive_extract_sender.send(ArchiveExtractionRequest::MoveSingle {
                    paths: vec![path],
                    dest_folder,
                    source_folder,
                    moved_dest,
                }) {
                    Ok(()) => return HandlerCompletion::DispatchedAsync,
                    Err(e) => {
                        log::warn!("[FileOps] Failed to dispatch archive extraction: {}", e);
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
                        return HandlerCompletion::CompletedSynchronously;
                    }
                }
            }

            let moved_dest = known_exact_move_dest(&path, &dest_folder);
            // Prefer IFileOperation for all copy/move paths. It matches modern
            // Explorer behavior better than SHFileOperationW, especially for
            // virtual filesystem providers such as Cryptomator.
            let success = shell_operations::move_item_with_file_op(&path, &dest_folder, hwnd.0);
            log::debug!("[FileOps] handle_move result: success={}", success);

            if success {
                if let Some(src) = source_folder {
                    let _ = result_sender.send(FileOperationResult::MoveCompleted {
                        source_folder: src,
                        dest_folder,
                        source_path: path,
                        moved_dest,
                    });
                }
            } else {
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
            }
            HandlerCompletion::CompletedSynchronously
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Move blocked: {}", err);
            HandlerCompletion::CompletedSynchronously
        }
    }
}

/// Moves a regular file using a create-new destination handle. This intentionally
/// avoids Shell conflict dialogs and never replaces an existing destination file.
pub(super) fn handle_organizer_move(
    path: PathBuf,
    dest_folder: PathBuf,
    rule_id: i64,
    activation: std::sync::Arc<std::sync::atomic::AtomicBool>,
    result_sender: &Sender<FileOperationResult>,
) {
    if !activation.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    let valid_path = sanitize_operation_path(&path);
    let valid_destination = sanitize_operation_path(&dest_folder);
    let (Ok(path), Ok(dest_folder)) = (valid_path, valid_destination) else {
        let _ = result_sender.send(FileOperationResult::OrganizerMoveFailed {
            rule_id,
            path,
            message: rust_i18n::t!("organizer.error_security_path").to_string(),
        });
        return;
    };

    let Some(source_folder) = path.parent().map(Path::to_path_buf) else {
        return;
    };
    let Some(file_name) = path.file_name() else {
        return;
    };
    let moved_dest = dest_folder.join(file_name);

    if !path.is_file() || !dest_folder.is_dir() {
        let _ = result_sender.send(FileOperationResult::OrganizerMoveFailed {
            rule_id,
            path,
            message: rust_i18n::t!("organizer.error_file_or_destination_unavailable").to_string(),
        });
        return;
    }
    if moved_dest.exists() {
        let _ = result_sender.send(FileOperationResult::OrganizerMoveSkipped { rule_id, path });
        return;
    }

    match shell_operations::move_file_without_replace(&path, &moved_dest) {
        Ok(()) => {
            let _ = result_sender.send(FileOperationResult::OrganizerMoveCompleted {
                rule_id,
                source_folder,
                dest_folder,
                source_path: path,
                moved_dest,
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = result_sender.send(FileOperationResult::OrganizerMoveSkipped { rule_id, path });
        }
        Err(error) => {
            let _ = result_sender.send(FileOperationResult::OrganizerMoveFailed {
                rule_id,
                path,
                message: error.to_string(),
            });
        }
    }
}

pub(super) fn handle_copy_batch(
    paths: Vec<PathBuf>,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
    archive_extract_sender: &Sender<ArchiveExtractionRequest>,
) -> HandlerCompletion {
    let valid_paths = sanitize_operation_paths(&paths);
    let valid_dest = sanitize_operation_path(&dest_folder);
    match (valid_paths, valid_dest) {
        (Ok(paths), Ok(dest_folder)) => {
            let has_virtual_path = paths.iter().any(|p| path_requires_native_extraction(p));
            let native_ok = archive_extract::has_native_support(&paths);
            log::debug!(
                "[FileOps] handle_copy_batch: {} paths, has_virtual={}, native_support={}",
                paths.len(),
                has_virtual_path,
                native_ok
            );
            for p in &paths {
                log::debug!("[FileOps]   batch path: {}", p.display());
            }

            let copied_dests = known_exact_new_copy_dests(&paths, &dest_folder, has_virtual_path);

            if has_virtual_path && native_ok {
                log::debug!(
                    "[FileOps] Dispatching native archive extraction for batch copy ({} files)",
                    paths.len()
                );
                match archive_extract_sender.send(ArchiveExtractionRequest::Copy {
                    paths,
                    dest_folder,
                    copied_dests,
                }) {
                    Ok(()) => return HandlerCompletion::DispatchedAsync,
                    Err(e) => {
                        log::warn!("[FileOps] Failed to dispatch archive extraction: {}", e);
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
                        return HandlerCompletion::CompletedSynchronously;
                    }
                }
            }

            // Prefer IFileOperation for all copy/move paths. It matches modern
            // Explorer behavior better than SHFileOperationW, especially for
            // virtual filesystem providers such as Cryptomator.
            let success = shell_operations::copy_items_with_file_op(&paths, &dest_folder, hwnd.0);
            log::debug!("[FileOps] handle_copy_batch result: success={}", success);

            if success {
                let _ = result_sender.send(FileOperationResult::CopyCompleted {
                    dest_folder,
                    copied_dests,
                });
            } else {
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
            }
            HandlerCompletion::CompletedSynchronously
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Copy batch blocked: {}", err);
            HandlerCompletion::CompletedSynchronously
        }
    }
}

pub(super) fn handle_move_batch(
    paths: Vec<PathBuf>,
    dest_folder: PathBuf,
    hwnd: SendHwnd,
    result_sender: &Sender<FileOperationResult>,
    archive_extract_sender: &Sender<ArchiveExtractionRequest>,
) -> HandlerCompletion {
    let (archive_paths, regular_paths) = split_virtual_archive_paths(paths);
    if !archive_paths.is_empty() {
        // Windows Shell cannot move items out of a compressed-folder view.
        // Copy archive entries while preserving true move semantics for any
        // regular filesystem items that happened to share the same batch.
        if regular_paths.is_empty() {
            log::debug!(
                "[FileOps] handle_move_batch downgraded to copy (paths inside archive): {} items",
                archive_paths.len()
            );
            return handle_copy_batch(
                archive_paths,
                dest_folder,
                hwnd,
                result_sender,
                archive_extract_sender,
            );
        }

        log::debug!(
            "[FileOps] handle_move_batch split mixed batch: {} archive entries copied, {} regular items moved",
            archive_paths.len(),
            regular_paths.len()
        );

        let move_completion = handle_move_batch(
            regular_paths,
            dest_folder.clone(),
            hwnd,
            result_sender,
            archive_extract_sender,
        );
        let copy_completion = handle_copy_batch(
            archive_paths,
            dest_folder,
            hwnd,
            result_sender,
            archive_extract_sender,
        );

        return if matches!(move_completion, HandlerCompletion::DispatchedAsync)
            || matches!(copy_completion, HandlerCompletion::DispatchedAsync)
        {
            HandlerCompletion::DispatchedAsync
        } else {
            HandlerCompletion::CompletedSynchronously
        };
    }

    let paths = regular_paths;

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

            let has_virtual_path = paths.iter().any(|p| path_requires_native_extraction(p));
            let native_ok = archive_extract::has_native_support(&paths);
            log::debug!(
                "[FileOps] handle_move_batch: {} paths, has_virtual={}, native_support={}",
                paths.len(),
                has_virtual_path,
                native_ok
            );

            if has_virtual_path && native_ok {
                log::debug!(
                    "[FileOps] Dispatching native archive extraction for batch move ({} files)",
                    paths.len()
                );
                let moved_files = paths.clone();
                let known_moved_pairs = known_exact_move_pairs(&paths, &dest_folder);
                match archive_extract_sender.send(ArchiveExtractionRequest::MoveBatch {
                    paths,
                    dest_folder,
                    source_folders: source_folders.into_iter().collect(),
                    moved_files,
                    known_moved_pairs,
                }) {
                    Ok(()) => return HandlerCompletion::DispatchedAsync,
                    Err(e) => {
                        log::warn!("[FileOps] Failed to dispatch archive extraction: {}", e);
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
                        return HandlerCompletion::CompletedSynchronously;
                    }
                }
            }

            let known_moved_pairs = known_exact_move_pairs(&paths, &dest_folder);
            // Prefer IFileOperation for all copy/move paths. It matches modern
            // Explorer behavior better than SHFileOperationW, especially for
            // virtual filesystem providers such as Cryptomator.
            let success = shell_operations::move_items_with_file_op(&paths, &dest_folder, hwnd.0);
            log::debug!("[FileOps] handle_move_batch result: success={}", success);

            if success && !source_folders.is_empty() {
                let _ = result_sender.send(FileOperationResult::MoveBatchCompleted {
                    source_folders: source_folders.into_iter().collect(),
                    dest_folder,
                    moved_files: paths,
                    known_moved_pairs,
                });
            } else if !success {
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
            }
            HandlerCompletion::CompletedSynchronously
        }
        (Err(err), _) | (_, Err(err)) => {
            log::warn!("[SECURITY] Move batch blocked: {}", err);
            HandlerCompletion::CompletedSynchronously
        }
    }
}

fn known_exact_move_dest(path: &Path, dest_folder: &Path) -> Option<PathBuf> {
    let candidate = path.file_name().map(|name| dest_folder.join(name))?;
    (!candidate.exists()).then_some(candidate)
}

fn known_exact_move_pairs(paths: &[PathBuf], dest_folder: &Path) -> Vec<(PathBuf, PathBuf)> {
    paths
        .iter()
        .filter_map(|path| {
            known_exact_move_dest(path, dest_folder).map(|dest| (path.clone(), dest))
        })
        .collect()
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
                let _ = result_sender.send(FileOperationResult::OperationFailed {
                    message: rust_i18n::t!("operations.error_cancelled").to_string(),
                });
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
            log::warn!(
                "[PROPERTIES] Failed to show properties for {}: {:?}",
                path.display(),
                e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_exact_new_copy_dests_includes_new_folder_dest() {
        let src_parent = tempfile::tempdir().expect("create source parent");
        let dest_parent = tempfile::tempdir().expect("create dest parent");
        let src_folder = src_parent.path().join("videos");
        std::fs::create_dir(&src_folder).expect("create source folder");

        let dests = known_exact_new_copy_dests(
            std::slice::from_ref(&src_folder),
            dest_parent.path(),
            false,
        );

        assert_eq!(dests, vec![dest_parent.path().join("videos")]);
    }

    #[test]
    fn known_exact_new_copy_dests_skips_existing_dest() {
        let src_parent = tempfile::tempdir().expect("create source parent");
        let dest_parent = tempfile::tempdir().expect("create dest parent");
        let src_folder = src_parent.path().join("videos");
        let existing_dest = dest_parent.path().join("videos");
        std::fs::create_dir(&src_folder).expect("create source folder");
        std::fs::create_dir(&existing_dest).expect("create existing dest folder");

        let dests = known_exact_new_copy_dests(&[src_folder], dest_parent.path(), false);

        assert!(dests.is_empty());
    }

    #[test]
    fn known_exact_move_pairs_include_only_non_existing_destinations() {
        let src_parent = tempfile::tempdir().expect("create source parent");
        let dest_parent = tempfile::tempdir().expect("create dest parent");
        let src_a = src_parent.path().join("a.txt");
        let src_b = src_parent.path().join("b.txt");
        std::fs::write(&src_a, b"a").expect("create source a");
        std::fs::write(&src_b, b"b").expect("create source b");
        std::fs::write(dest_parent.path().join("b.txt"), b"existing")
            .expect("create conflicting destination");

        let pairs = known_exact_move_pairs(&[src_a.clone(), src_b], dest_parent.path());

        assert_eq!(pairs, vec![(src_a, dest_parent.path().join("a.txt"))]);
    }

    #[test]
    fn organizer_move_never_replaces_an_existing_destination() {
        let source_parent = tempfile::tempdir().expect("create source parent");
        let destination_parent = tempfile::tempdir().expect("create destination parent");
        let source = source_parent.path().join("report.pdf");
        let destination = destination_parent.path().join("report.pdf");
        std::fs::write(&source, b"source").expect("create source file");
        std::fs::write(&destination, b"destination").expect("create destination file");

        let result = shell_operations::move_file_without_replace(&source, &destination);

        assert!(matches!(
            result.map_err(|error| error.kind()),
            Err(std::io::ErrorKind::AlreadyExists)
        ));
        assert_eq!(std::fs::read(&source).expect("source remains"), b"source");
        assert_eq!(
            std::fs::read(&destination).expect("destination remains"),
            b"destination"
        );
    }

    #[test]
    fn organizer_move_preserves_the_move_semantics() {
        let source_parent = tempfile::tempdir().expect("create source parent");
        let destination_parent = tempfile::tempdir().expect("create destination parent");
        let source = source_parent.path().join("report.pdf");
        let destination = destination_parent.path().join("report.pdf");
        std::fs::write(&source, b"contents").expect("create source file");
        let created_at = std::fs::metadata(&source)
            .expect("source metadata")
            .created()
            .expect("source creation time");

        shell_operations::move_file_without_replace(&source, &destination).expect("move file");

        assert!(!source.exists());
        assert_eq!(
            std::fs::read(&destination).expect("destination contents"),
            b"contents"
        );
        assert_eq!(
            std::fs::metadata(&destination)
                .expect("destination metadata")
                .created()
                .expect("destination creation time"),
            created_at
        );
    }

    #[test]
    fn split_virtual_archive_paths_separates_archive_entries_from_regular_paths() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive_a_root = dir.path().join("archive.zip");
        let archive_b_root = dir.path().join("data.7z");
        std::fs::write(&archive_a_root, b"zip placeholder").expect("create zip file");
        std::fs::write(&archive_b_root, b"7z placeholder").expect("create 7z file");

        let archive_a = archive_a_root.join("inner").join("file.txt");
        let archive_b = archive_b_root.join("nested").join("sub");
        let regular_a = dir.path().join("file.zip");
        std::fs::write(&regular_a, b"zip root placeholder").expect("create regular archive root");
        let regular_b = PathBuf::from(r"C:\Windows\notepad.exe");
        let paths = vec![
            archive_a.clone(),
            regular_a.clone(),
            archive_b.clone(),
            regular_b.clone(),
        ];

        let (archive_paths, regular_paths) = split_virtual_archive_paths(paths);

        assert_eq!(archive_paths, vec![archive_a, archive_b]);
        assert_eq!(regular_paths, vec![regular_a, regular_b]);
    }

    #[test]
    fn split_virtual_archive_paths_keeps_archive_roots_with_regular_paths() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive_root = dir.path().join("file.zip");
        let archive_named_dir = dir.path().join("folder.zip");
        std::fs::write(&archive_root, b"zip placeholder").expect("create archive root");
        std::fs::create_dir(&archive_named_dir).expect("create archive-named directory");

        let paths = vec![
            archive_root,
            archive_named_dir.join("file.txt"),
            PathBuf::from(r"C:\Windows\notepad.exe"),
            PathBuf::from(r"D:\videos\movie.mp4"),
        ];

        let (archive_paths, regular_paths) = split_virtual_archive_paths(paths.clone());

        assert!(archive_paths.is_empty());
        assert_eq!(regular_paths, paths);
    }
}
