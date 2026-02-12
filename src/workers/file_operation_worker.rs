//! Worker thread for Windows Shell file operations.
//! Ensures COM is initialized as STA (COINIT_APARTMENTTHREADED) for correct shell behavior.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

use crate::infrastructure::security::{
    sanitize_path_with_local_drive_fallback, sanitize_unc_path, SecurityConfig,
};
use crate::infrastructure::windows::recycle_bin;
use crate::infrastructure::windows::shell_operations;

/// Results sent back from the worker to the UI.
pub enum FileOperationResult {
    /// Generic notification that a file operation finished
    Finished,
    /// Specifically for Recycle Bin operations to trigger targeted refresh
    RecycleBinChanged,
    /// Restore operation completed - original folders need refresh
    RestoreCompleted { parent_folders: Vec<PathBuf> },
    /// Delete operation completed - parent folders need refresh
    DeleteCompleted { parent_folders: Vec<PathBuf> },
    /// Move operation completed - source folder needs refresh in all tabs, dest needs reload if active
    MoveCompleted {
        source_folder: PathBuf,
        dest_folder: PathBuf,
    },
    /// Batch move completed - multiple source folders need refresh
    MoveBatchCompleted {
        source_folders: Vec<PathBuf>,
        dest_folder: PathBuf,
        /// The actual files/folders that were moved (for folder cover invalidation)
        moved_files: Vec<PathBuf>,
    },
    /// Copy operation completed - dest folder needs reload if active
    CopyCompleted { dest_folder: PathBuf },
    RenameCompleted {
        path: PathBuf,
        new_name: String,
        parent_folder: PathBuf,
    },
}

/// Transparent wrapper for HWND to make it Send.
/// SAFETY: HWNDs are globally valid in Windows and can be used from any thread.
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);
unsafe impl Send for SendHwnd {}

/// Requests that can be sent to the file operation worker.
pub enum FileOperationRequest {
    Delete {
        paths: Vec<PathBuf>,
        hwnd: SendHwnd,
    },
    Rename {
        path: PathBuf,
        new_name: String,
        hwnd: SendHwnd,
    },
    Copy {
        path: PathBuf,
        dest_folder: PathBuf,
        hwnd: SendHwnd,
    },
    Move {
        path: PathBuf,
        dest_folder: PathBuf,
        hwnd: SendHwnd,
    },
    /// Batch copy: all files in a single Shell operation (single progress dialog)
    CopyBatch {
        paths: Vec<PathBuf>,
        dest_folder: PathBuf,
        hwnd: SendHwnd,
    },
    /// Batch move: all files in a single Shell operation (single progress dialog)
    MoveBatch {
        paths: Vec<PathBuf>,
        dest_folder: PathBuf,
        hwnd: SendHwnd,
    },
    RestoreFromRecycleBin {
        items: Vec<(PathBuf, PathBuf)>,
    },
    DeletePermanently {
        physical_paths: Vec<PathBuf>,
        hwnd: SendHwnd,
    },
    EmptyRecycleBin {
        hwnd: SendHwnd,
    },
}

impl FileOperationRequest {
    // Helper to wrap HWND
    pub fn delete(paths: Vec<PathBuf>, hwnd: HWND) -> Self {
        Self::Delete {
            paths,
            hwnd: SendHwnd(hwnd),
        }
    }
    pub fn rename(path: PathBuf, new_name: String, hwnd: HWND) -> Self {
        Self::Rename {
            path,
            new_name,
            hwnd: SendHwnd(hwnd),
        }
    }
    pub fn copy(path: PathBuf, dest_folder: PathBuf, hwnd: HWND) -> Self {
        Self::Copy {
            path,
            dest_folder,
            hwnd: SendHwnd(hwnd),
        }
    }
    pub fn file_move(path: PathBuf, dest_folder: PathBuf, hwnd: HWND) -> Self {
        Self::Move {
            path,
            dest_folder,
            hwnd: SendHwnd(hwnd),
        }
    }
    pub fn copy_batch(paths: Vec<PathBuf>, dest_folder: PathBuf, hwnd: HWND) -> Self {
        Self::CopyBatch {
            paths,
            dest_folder,
            hwnd: SendHwnd(hwnd),
        }
    }
    pub fn move_batch(paths: Vec<PathBuf>, dest_folder: PathBuf, hwnd: HWND) -> Self {
        Self::MoveBatch {
            paths,
            dest_folder,
            hwnd: SendHwnd(hwnd),
        }
    }
}

fn operation_security_config() -> SecurityConfig {
    // Only allow drives that are actually mounted, detected via GetLogicalDrives().
    let mask = crate::infrastructure::windows::get_logical_drives_bitmask();
    let allowed_drives = if mask != 0 {
        (0u8..26)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| format!("{}:", (b'A' + i) as char))
            .collect()
    } else {
        vec!["C:".to_string()]
    };
    SecurityConfig {
        allowed_drives,
        // Windows commonly uses junctions/reparse points in valid user paths.
        allow_symlinks: true,
        ..SecurityConfig::default()
    }
}

fn should_bypass_sanitization(path: &Path) -> bool {
    let s = path.to_string_lossy();
    // Only true shell namespace paths (shell:, ::{GUID}) bypass sanitization.
    // UNC network paths now go through basic validation instead of bypassing entirely.
    s.starts_with("shell:") || crate::infrastructure::windows::is_shell_navigation_path(path, false)
}

/// Returns true for UNC network paths that need lightweight validation
/// instead of full drive-based sanitization.
fn is_unc_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if !s.starts_with(r"\\") {
        return false;
    }
    // \\?\C:\... is a local verbatim path, NOT UNC — handle via normal sanitization.
    // \\?\UNC\server\share is a verbatim UNC path.
    // \\server\share is a standard UNC path.
    match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.starts_with("UNC\\"),
        None => true,
    }
}

fn sanitize_operation_path(path: &Path) -> Result<PathBuf, String> {
    if should_bypass_sanitization(path) {
        return Ok(path.to_path_buf());
    }
    if is_unc_path(path) {
        return sanitize_unc_path(path).map_err(|e| e.to_string());
    }

    sanitize_path_with_local_drive_fallback(path, &operation_security_config())
        .map_err(|e| format!("Security validation failed for '{}': {}", path.display(), e))
}

fn sanitize_operation_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    paths.iter().map(|p| sanitize_operation_path(p)).collect()
}

/// Starts the file operation worker thread.
pub fn start_file_operation_worker(
    receiver: Receiver<FileOperationRequest>,
    result_sender: std::sync::mpsc::Sender<FileOperationResult>,
) {
    std::thread::spawn(move || {
        // Initialize COM as Single-Threaded Apartment (STA)
        // This is critical for shell progress dialogs and proper COM behavior.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        while let Ok(request) = receiver.recv() {
            match request {
                FileOperationRequest::Delete { paths, hwnd } => {
                    match sanitize_operation_paths(&paths) {
                        Ok(valid_paths) => {
                            if valid_paths.is_empty() {
                                // no-op
                            } else {
                                let _ =
                                    shell_operations::delete_items_with_shell(&valid_paths, hwnd.0);
                                let mut parents = HashSet::new();
                                for path in &valid_paths {
                                    if let Some(parent) = path.parent() {
                                        parents.insert(parent.to_path_buf());
                                    }
                                }
                                if !parents.is_empty() {
                                    let _ =
                                        result_sender.send(FileOperationResult::DeleteCompleted {
                                            parent_folders: parents.into_iter().collect(),
                                        });
                                }
                                let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                            }
                        }
                        Err(err) => {
                            eprintln!("[SECURITY] Delete blocked: {}", err);
                        }
                    }
                }
                FileOperationRequest::Rename {
                    path,
                    new_name,
                    hwnd,
                } => {
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
                    if invalid_chars
                        || new_name == "."
                        || new_name == ".."
                        || new_name.ends_with('.')
                        || new_name.ends_with(' ')
                        || crate::infrastructure::security::is_windows_reserved_name(base_name)
                    {
                        eprintln!(
                            "[SECURITY] Rename blocked: invalid target name '{}'",
                            new_name
                        );
                    } else {
                        match sanitize_operation_path(&path) {
                            Ok(valid_path) => {
                                let success = shell_operations::rename_item_with_shell(
                                    &valid_path,
                                    &new_name,
                                    hwnd.0,
                                );
                                if success {
                                    if let Some(parent) =
                                        valid_path.parent().map(|p| p.to_path_buf())
                                    {
                                        let _ = result_sender.send(
                                            FileOperationResult::RenameCompleted {
                                                path: valid_path,
                                                new_name: new_name.clone(),
                                                parent_folder: parent,
                                            },
                                        );
                                    }
                                }
                            }
                            Err(err) => {
                                eprintln!("[SECURITY] Rename blocked: {}", err);
                            }
                        }
                    }
                }
                FileOperationRequest::Copy {
                    path,
                    dest_folder,
                    hwnd,
                } => {
                    let valid_path = sanitize_operation_path(&path);
                    let valid_dest = sanitize_operation_path(&dest_folder);
                    match (valid_path, valid_dest) {
                        (Ok(path), Ok(dest_folder)) => {
                            if crate::infrastructure::windows::is_shell_navigation_path(
                                &path, false,
                            ) {
                                let _ = shell_operations::copy_item_with_file_op(
                                    &path,
                                    &dest_folder,
                                    hwnd.0,
                                );
                            } else {
                                let _ = shell_operations::copy_item_with_shell(
                                    &path,
                                    &dest_folder,
                                    hwnd.0,
                                );
                            }
                            let _ = result_sender
                                .send(FileOperationResult::CopyCompleted { dest_folder });
                        }
                        (Err(err), _) | (_, Err(err)) => {
                            eprintln!("[SECURITY] Copy blocked: {}", err);
                        }
                    }
                }
                FileOperationRequest::Move {
                    path,
                    dest_folder,
                    hwnd,
                } => {
                    let valid_path = sanitize_operation_path(&path);
                    let valid_dest = sanitize_operation_path(&dest_folder);
                    match (valid_path, valid_dest) {
                        (Ok(path), Ok(dest_folder)) => {
                            // Capture source folder before move
                            let source_folder = path.parent().map(|p| p.to_path_buf());
                            // Use IFileOperation for virtual paths (like items inside archives)
                            let success =
                                if crate::infrastructure::windows::is_shell_navigation_path(
                                    &path, false,
                                ) {
                                    shell_operations::move_item_with_file_op(
                                        &path,
                                        &dest_folder,
                                        hwnd.0,
                                    )
                                } else {
                                    shell_operations::move_item_with_shell(
                                        &path,
                                        &dest_folder,
                                        hwnd.0,
                                    )
                                };

                            if success {
                                if let Some(src) = source_folder {
                                    let _ =
                                        result_sender.send(FileOperationResult::MoveCompleted {
                                            source_folder: src,
                                            dest_folder,
                                        });
                                }
                            }
                        }
                        (Err(err), _) | (_, Err(err)) => {
                            eprintln!("[SECURITY] Move blocked: {}", err);
                        }
                    }
                }
                FileOperationRequest::CopyBatch {
                    paths,
                    dest_folder,
                    hwnd,
                } => {
                    let valid_paths = sanitize_operation_paths(&paths);
                    let valid_dest = sanitize_operation_path(&dest_folder);
                    match (valid_paths, valid_dest) {
                        (Ok(paths), Ok(dest_folder)) => {
                            let has_virtual_path = paths.iter().any(|p| {
                                crate::infrastructure::windows::is_shell_navigation_path(p, false)
                            });

                            let success = if has_virtual_path {
                                shell_operations::copy_items_with_file_op(
                                    &paths,
                                    &dest_folder,
                                    hwnd.0,
                                )
                            } else {
                                shell_operations::copy_items_with_shell(
                                    &paths,
                                    &dest_folder,
                                    hwnd.0,
                                )
                            };

                            if success {
                                let _ = result_sender
                                    .send(FileOperationResult::CopyCompleted { dest_folder });
                            }
                        }
                        (Err(err), _) | (_, Err(err)) => {
                            eprintln!("[SECURITY] Copy batch blocked: {}", err);
                        }
                    }
                }
                FileOperationRequest::MoveBatch {
                    paths,
                    dest_folder,
                    hwnd,
                } => {
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

                            let has_virtual_path = paths.iter().any(|p| {
                                crate::infrastructure::windows::is_shell_navigation_path(p, false)
                            });

                            let success = if has_virtual_path {
                                shell_operations::move_items_with_file_op(
                                    &paths,
                                    &dest_folder,
                                    hwnd.0,
                                )
                            } else {
                                shell_operations::move_items_with_shell(
                                    &paths,
                                    &dest_folder,
                                    hwnd.0,
                                )
                            };

                            if success && !source_folders.is_empty() {
                                let _ =
                                    result_sender.send(FileOperationResult::MoveBatchCompleted {
                                        source_folders: source_folders.into_iter().collect(),
                                        dest_folder,
                                        moved_files: paths,
                                    });
                            }
                        }
                        (Err(err), _) | (_, Err(err)) => {
                            eprintln!("[SECURITY] Move batch blocked: {}", err);
                        }
                    }
                }
                FileOperationRequest::RestoreFromRecycleBin { items } => {
                    let mut parents = HashSet::new();
                    for (physical_path, original_path) in items {
                        let valid_physical = sanitize_operation_path(&physical_path);
                        let valid_original = sanitize_operation_path(&original_path);
                        match (valid_physical, valid_original) {
                            (Ok(physical_path), Ok(original_path)) => {
                                if let Some(parent) = original_path.parent() {
                                    parents.insert(parent.to_path_buf());
                                }
                                let _ = recycle_bin::restore_from_recycle_bin(
                                    &physical_path,
                                    &original_path,
                                );
                            }
                            (Err(err), _) | (_, Err(err)) => {
                                eprintln!("[SECURITY] Restore blocked: {}", err);
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
                FileOperationRequest::DeletePermanently {
                    physical_paths,
                    hwnd,
                } => match sanitize_operation_paths(&physical_paths) {
                    Ok(valid_paths) => {
                        for path in &valid_paths {
                            if recycle_bin::delete_permanently(path, hwnd.0).is_err() {
                                break;
                            }
                        }
                        let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                    }
                    Err(err) => {
                        eprintln!("[SECURITY] Permanent delete blocked: {}", err);
                    }
                },
                FileOperationRequest::EmptyRecycleBin { hwnd } => {
                    let _ = recycle_bin::empty_recycle_bin(hwnd.0);
                    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                }
            }

            // Notify general completion for other operations.
            let _ = result_sender.send(FileOperationResult::Finished);
        }

        unsafe {
            CoUninitialize();
        }
    });
}
