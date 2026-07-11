//! Worker thread for Windows Shell file operations.
//! Ensures COM is initialized as STA (COINIT_APARTMENTTHREADED) for correct shell behavior.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use windows::Win32::Foundation::HWND;

mod handlers;

use crate::infrastructure::diagnostic_logger::{diag_error, field_label};
use crate::infrastructure::security::{
    classify_shell_namespace_path, sanitize_path_with_local_drive_fallback, sanitize_unc_path,
    SecurityConfig,
};
use crate::infrastructure::windows::ComScope;
use crate::workers::archive_extraction_worker::ArchiveExtractionRequest;

pub enum FileOperationResult {
    /// Generic notification that a file operation finished
    Finished,
    /// Operation finished, but a specific completion handler already updated the affected views.
    FinishedNoRefresh,
    /// Specifically for Recycle Bin operations to trigger targeted refresh
    RecycleBinChanged,
    /// Restore operation completed - original folders need refresh
    RestoreCompleted {
        parent_folders: Vec<PathBuf>,
    },
    /// Delete operation completed - parent folders need refresh
    DeleteCompleted {
        parent_folders: Vec<PathBuf>,
        deleted_paths: Vec<PathBuf>,
    },
    /// Move operation completed - source folder needs refresh in all tabs, dest needs reload if active
    MoveCompleted {
        source_folder: PathBuf,
        dest_folder: PathBuf,
        /// Original source path of the moved item.
        source_path: PathBuf,
        /// Destination path of the moved item (for write-activity cache clearing).
        moved_dest: Option<PathBuf>,
    },
    /// Batch move completed - multiple source folders need refresh
    MoveBatchCompleted {
        source_folders: Vec<PathBuf>,
        dest_folder: PathBuf,
        /// The actual files/folders that were moved (for folder cover invalidation)
        moved_files: Vec<PathBuf>,
        /// Source/destination pairs whose destination was known to be unambiguous.
        known_moved_pairs: Vec<(PathBuf, PathBuf)>,
    },
    /// A background organizer move completed without allowing replacement of an existing file.
    OrganizerMoveCompleted {
        rule_id: i64,
        source_folder: PathBuf,
        dest_folder: PathBuf,
        source_path: PathBuf,
        moved_dest: PathBuf,
    },
    OrganizerMoveSkipped {
        rule_id: i64,
        path: PathBuf,
    },
    OrganizerMoveFailed {
        rule_id: i64,
        path: PathBuf,
        message: String,
    },
    /// Copy operation completed - dest folder needs reload if active
    CopyCompleted {
        dest_folder: PathBuf,
        /// Known exact destination files/folders (for write-activity cache clearing).
        copied_dests: Vec<PathBuf>,
    },
    RenameCompleted {
        path: PathBuf,
        new_name: String,
        parent_folder: PathBuf,
    },
    RenameBatchProgress {
        completed: usize,
        total: usize,
        current_name: String,
    },
    RenameBatchCompleted {
        /// Number of items successfully renamed.
        count: usize,
    },
    DriveRenameCompleted {
        drive_path: PathBuf,
        new_label: String,
    },
    DriveRenameFailed {
        drive_path: PathBuf,
        error: String,
        cancelled: bool,
    },
    /// A file operation failed or was cancelled by the user.
    OperationFailed {
        message: String,
    },
}

enum CompletionBehavior {
    SendFinished,
    SendFinishedNoRefresh,
    NoFinished,
}

/// Transparent wrapper for HWND to make it Send.
/// SAFETY: HWNDs are globally valid in Windows and can be used from any thread.
#[derive(Clone, Copy)]
pub(crate) struct SendHwnd(pub(crate) HWND);
unsafe impl Send for SendHwnd {}

/// Requests that can be sent to the file operation worker.
#[allow(dead_code)] // Copy/Move variants intentionally kept for single-file operations
pub(crate) enum FileOperationRequest {
    Delete {
        paths: Vec<PathBuf>,
        hwnd: SendHwnd,
    },
    Rename {
        path: PathBuf,
        new_name: String,
        hwnd: SendHwnd,
    },
    RenameBatch {
        renames: Vec<(PathBuf, String)>,
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
    /// Move one regular file without replacing an existing destination file.
    OrganizerMove {
        path: PathBuf,
        dest_folder: PathBuf,
        rule_id: i64,
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
    /// Show Windows Properties dialog for a set of paths.
    /// Fire-and-forget: SHObjectProperties opens a modeless dialog.
    ShowProperties {
        paths: Vec<PathBuf>,
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
    pub fn delete_permanently(paths: Vec<PathBuf>, hwnd: HWND) -> Self {
        Self::DeletePermanently {
            physical_paths: paths,
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
    pub fn rename_batch(renames: Vec<(PathBuf, String)>, hwnd: HWND) -> Self {
        Self::RenameBatch {
            renames,
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
    pub fn show_properties(paths: Vec<PathBuf>, hwnd: HWND) -> Self {
        Self::ShowProperties {
            paths,
            hwnd: SendHwnd(hwnd),
        }
    }

    /// Replaces the HWND in all variants that carry one.
    /// Used by the worker thread to substitute the UI-thread HWND with a
    /// worker-thread proxy HWND, ensuring Shell progress dialogs are owned by
    /// a window on the SAME thread as the `SHFileOperationW` call.
    /// This avoids cross-thread `SendMessage` marshaling that can cause
    /// UI thread starvation and total app freeze during long file operations.
    fn substitute_hwnd(self, hwnd: SendHwnd) -> Self {
        match self {
            Self::Delete { paths, .. } => Self::Delete { paths, hwnd },
            Self::Rename { path, new_name, .. } => Self::Rename {
                path,
                new_name,
                hwnd,
            },
            Self::RenameBatch { renames, .. } => Self::RenameBatch { renames, hwnd },
            Self::Copy {
                path, dest_folder, ..
            } => Self::Copy {
                path,
                dest_folder,
                hwnd,
            },
            Self::Move {
                path, dest_folder, ..
            } => Self::Move {
                path,
                dest_folder,
                hwnd,
            },
            Self::OrganizerMove {
                path,
                dest_folder,
                rule_id,
            } => Self::OrganizerMove {
                path,
                dest_folder,
                rule_id,
            },
            Self::CopyBatch {
                paths, dest_folder, ..
            } => Self::CopyBatch {
                paths,
                dest_folder,
                hwnd,
            },
            Self::MoveBatch {
                paths, dest_folder, ..
            } => Self::MoveBatch {
                paths,
                dest_folder,
                hwnd,
            },
            Self::RestoreFromRecycleBin { items } => Self::RestoreFromRecycleBin { items },
            Self::DeletePermanently { physical_paths, .. } => Self::DeletePermanently {
                physical_paths,
                hwnd,
            },
            Self::EmptyRecycleBin { .. } => Self::EmptyRecycleBin { hwnd },
            Self::ShowProperties { paths, hwnd } => Self::ShowProperties { paths, hwnd },
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

fn is_explicit_shell_namespace_path(path: &Path) -> bool {
    classify_shell_namespace_path(path).is_some()
}

fn should_bypass_sanitization(path: &Path) -> bool {
    // Only explicit shell namespace identifiers bypass sanitization.
    // Archive-like filesystem paths no longer bypass validation.
    is_explicit_shell_namespace_path(path)
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
pub(crate) fn start_file_operation_worker(
    receiver: Receiver<FileOperationRequest>,
    result_sender: std::sync::mpsc::Sender<FileOperationResult>,
    archive_extract_sender: std::sync::mpsc::Sender<ArchiveExtractionRequest>,
) {
    let spawn_result = crate::spawn_named("file-op-worker", move || {
        // Initialize COM as Single-Threaded Apartment (STA)
        // RAII guard ensures CoUninitialize even on panic.
        let _com = ComScope::sta();

        // Create a proxy HWND ON THE WORKER THREAD so that Shell progress
        // dialogs (shown by SHFileOperationW / IFileOperation) are owned by a
        // window on the SAME thread as the blocking call.
        //
        // Previously the proxy HWND was created on the UI thread and passed
        // to the worker.  When SHFileOperationW disabled that cross-thread
        // owner via EnableWindow(), Windows marshaled the WM_ENABLE message
        // to the UI thread with SendMessage.  If the UI thread was busy
        // rendering, the worker blocked — but more critically, the Shell's
        // internal modal message loop could re-enter the UI thread's message
        // pump and cause the entire window to freeze (total UI lockup for
        // the duration of the file operation).
        //
        // With a same-thread proxy, EnableWindow is a direct call (no
        // cross-thread marshaling) and the Shell's modal loop stays entirely
        // on the worker thread.
        let worker_proxy_hwnd =
            crate::infrastructure::windows::shell_operations::create_shell_op_proxy_window();
        if worker_proxy_hwnd.is_none() {
            log::warn!(
                "[FileOpWorker] Proxy window creation failed on worker thread; \
                 falling back to caller-provided HWND (may cause UI freeze)"
            );
        }

        while let Ok(request) = receiver.recv() {
            // Substitute the UI-thread HWND with the worker-thread proxy.
            // Falls back to the original HWND if proxy creation failed.
            let request = match worker_proxy_hwnd {
                Some(proxy) => request.substitute_hwnd(SendHwnd(proxy)),
                None => request,
            };

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match request {
                    FileOperationRequest::Delete { paths, hwnd } => {
                        handlers::handle_delete(paths, hwnd, &result_sender);
                    }
                    FileOperationRequest::Rename {
                        path,
                        new_name,
                        hwnd,
                    } => {
                        handlers::handle_rename(path, new_name, hwnd, &result_sender);
                    }
                    FileOperationRequest::RenameBatch { renames, hwnd } => {
                        handlers::handle_rename_batch(renames, hwnd, &result_sender);
                        return CompletionBehavior::SendFinishedNoRefresh;
                    }
                    FileOperationRequest::Copy {
                        path,
                        dest_folder,
                        hwnd,
                    } => {
                        let completion = handlers::handle_copy(
                            path,
                            dest_folder,
                            hwnd,
                            &result_sender,
                            &archive_extract_sender,
                        );
                        // handle_copy_completed already reloads the destination view.
                        return match completion {
                            handlers::HandlerCompletion::CompletedSynchronously => {
                                CompletionBehavior::SendFinishedNoRefresh
                            }
                            handlers::HandlerCompletion::DispatchedAsync => {
                                CompletionBehavior::NoFinished
                            }
                        };
                    }
                    FileOperationRequest::Move {
                        path,
                        dest_folder,
                        hwnd,
                    } => {
                        let completion = handlers::handle_move(
                            path,
                            dest_folder,
                            hwnd,
                            &result_sender,
                            &archive_extract_sender,
                        );
                        // handle_move_completed already reloads source + dest views.
                        return match completion {
                            handlers::HandlerCompletion::CompletedSynchronously => {
                                CompletionBehavior::SendFinishedNoRefresh
                            }
                            handlers::HandlerCompletion::DispatchedAsync => {
                                CompletionBehavior::NoFinished
                            }
                        };
                    }
                    FileOperationRequest::OrganizerMove {
                        path,
                        dest_folder,
                        rule_id,
                    } => {
                        handlers::handle_organizer_move(path, dest_folder, rule_id, &result_sender);
                        return CompletionBehavior::NoFinished;
                    }
                    FileOperationRequest::CopyBatch {
                        paths,
                        dest_folder,
                        hwnd,
                    } => {
                        let completion = handlers::handle_copy_batch(
                            paths,
                            dest_folder,
                            hwnd,
                            &result_sender,
                            &archive_extract_sender,
                        );
                        // handle_copy_completed already reloads the destination view.
                        return match completion {
                            handlers::HandlerCompletion::CompletedSynchronously => {
                                CompletionBehavior::SendFinishedNoRefresh
                            }
                            handlers::HandlerCompletion::DispatchedAsync => {
                                CompletionBehavior::NoFinished
                            }
                        };
                    }
                    FileOperationRequest::MoveBatch {
                        paths,
                        dest_folder,
                        hwnd,
                    } => {
                        let completion = handlers::handle_move_batch(
                            paths,
                            dest_folder,
                            hwnd,
                            &result_sender,
                            &archive_extract_sender,
                        );
                        // handle_move_batch_completed already reloads source + dest views.
                        return match completion {
                            handlers::HandlerCompletion::CompletedSynchronously => {
                                CompletionBehavior::SendFinishedNoRefresh
                            }
                            handlers::HandlerCompletion::DispatchedAsync => {
                                CompletionBehavior::NoFinished
                            }
                        };
                    }
                    FileOperationRequest::RestoreFromRecycleBin { items } => {
                        handlers::handle_restore_from_recycle_bin(items, &result_sender);
                    }
                    FileOperationRequest::DeletePermanently {
                        physical_paths,
                        hwnd,
                    } => handlers::handle_delete_permanently(physical_paths, hwnd, &result_sender),
                    FileOperationRequest::EmptyRecycleBin { hwnd } => {
                        handlers::handle_empty_recycle_bin(hwnd, &result_sender);
                    }
                    FileOperationRequest::ShowProperties { paths, hwnd } => {
                        handlers::handle_show_properties(paths, hwnd);
                        // No Finished message — fire-and-forget, dialog manages itself
                        return CompletionBehavior::NoFinished;
                    }
                }
                CompletionBehavior::SendFinished
            }));

            match result {
                Ok(CompletionBehavior::SendFinished) => {
                    // Notify general completion for other operations.
                    let _ = result_sender.send(FileOperationResult::Finished);
                }
                Ok(CompletionBehavior::SendFinishedNoRefresh) => {
                    let _ = result_sender.send(FileOperationResult::FinishedNoRefresh);
                }
                Ok(CompletionBehavior::NoFinished) => {}
                Err(e) => {
                    let (msg, panic_payload) = if let Some(s) = e.downcast_ref::<&str>() {
                        (s.to_string(), "str")
                    } else if let Some(s) = e.downcast_ref::<String>() {
                        (s.clone(), "string")
                    } else {
                        ("unknown".to_string(), "unknown")
                    };
                    log::error!("[FileOpWorker] worker thread panicked");
                    diag_error(
                        "file_operation_worker",
                        "worker_panic",
                        &[field_label("payload_kind", panic_payload)],
                    );
                    let _ =
                        result_sender.send(FileOperationResult::OperationFailed { message: msg });
                    let _ = result_sender.send(FileOperationResult::Finished);
                }
            }
        }
        // COM cleanup handled by _com (ComGuard) RAII Drop
    });

    if let Err(error) = spawn_result {
        log::error!("[FileOpWorker] failed to spawn worker thread: {}", error);
        diag_error("file_operation_worker", "spawn_failed", &[]);
    }
}

#[cfg(test)]
mod tests {
    use super::is_explicit_shell_namespace_path;
    use std::path::Path;

    #[test]
    fn shell_namespace_bypass_accepts_only_explicit_namespace_forms() {
        assert!(is_explicit_shell_namespace_path(Path::new(
            "shell:RecycleBinFolder"
        )));
        assert!(is_explicit_shell_namespace_path(Path::new(
            "::{645FF040-5081-101B-9F08-00AA002F954E}"
        )));
        assert!(is_explicit_shell_namespace_path(Path::new(
            r"\\?\::{645FF040-5081-101B-9F08-00AA002F954E}"
        )));

        assert!(!is_explicit_shell_namespace_path(Path::new(
            r"C:\Temp\file.txt"
        )));
        assert!(!is_explicit_shell_namespace_path(Path::new(
            r"C:\Temp\archive.zip\inside"
        )));
    }
}
