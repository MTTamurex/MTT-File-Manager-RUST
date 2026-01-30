//! Worker thread for Windows Shell file operations.
//! Ensures COM is initialized as STA (COINIT_APARTMENTTHREADED) for correct shell behavior.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

use crate::infrastructure::windows::recycle_bin;
use crate::infrastructure::windows::shell_operations;

/// Results sent back from the worker to the UI.
pub enum FileOperationResult {
    /// Generic notification that a file operation finished
    Finished,
    /// Specifically for Recycle Bin operations to trigger targeted refresh
    RecycleBinChanged,
    /// Delete operation completed - parent folders need refresh
    DeleteCompleted { parent_folders: Vec<PathBuf> },
    /// Move operation completed - source folder needs refresh in all tabs, dest needs reload if active
    MoveCompleted {
        source_folder: PathBuf,
        dest_folder: PathBuf,
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
    RestoreFromRecycleBin {
        items: Vec<(PathBuf, PathBuf)>,
    },
    DeletePermanently {
        physical_paths: Vec<PathBuf>,
    },
    EmptyRecycleBin,
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
                    let _ = shell_operations::delete_items_with_shell(&paths, hwnd.0);
                    let mut parents = HashSet::new();
                    for path in &paths {
                        if let Some(parent) = path.parent() {
                            parents.insert(parent.to_path_buf());
                        }
                    }
                    if !parents.is_empty() {
                        let _ = result_sender.send(FileOperationResult::DeleteCompleted {
                            parent_folders: parents.into_iter().collect(),
                        });
                    }
                    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                }
                FileOperationRequest::Rename {
                    path,
                    new_name,
                    hwnd,
                } => {
                    let success =
                        shell_operations::rename_item_with_shell(&path, &new_name, hwnd.0);
                    if success {
                        if let Some(parent) = path.parent() {
                            let _ = result_sender.send(FileOperationResult::RenameCompleted {
                                path: path.clone(),
                                new_name: new_name.clone(),
                                parent_folder: parent.to_path_buf(),
                            });
                        }
                    }
                }
                FileOperationRequest::Copy {
                    path,
                    dest_folder,
                    hwnd,
                } => {
                    if crate::infrastructure::windows::is_shell_navigation_path(&path, false) {
                        let _ =
                            shell_operations::copy_item_with_file_op(&path, &dest_folder, hwnd.0);
                    } else {
                        let _ = shell_operations::copy_item_with_shell(&path, &dest_folder, hwnd.0);
                    }
                    let _ = result_sender.send(FileOperationResult::CopyCompleted { dest_folder });
                }
                FileOperationRequest::Move {
                    path,
                    dest_folder,
                    hwnd,
                } => {
                    // Capture source folder before move
                    let source_folder = path.parent().map(|p| p.to_path_buf());
                    let success =
                        shell_operations::move_item_with_shell(&path, &dest_folder, hwnd.0);
                    // Notify source folder for cross-tab refresh
                    if success {
                        if let Some(src) = source_folder {
                            let _ = result_sender.send(FileOperationResult::MoveCompleted {
                                source_folder: src,
                                dest_folder,
                            });
                        }
                    }
                }
                FileOperationRequest::RestoreFromRecycleBin { items } => {
                    for (physical_path, original_path) in items {
                        let _ =
                            recycle_bin::restore_from_recycle_bin(&physical_path, &original_path);
                    }
                    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                }
                FileOperationRequest::DeletePermanently { physical_paths } => {
                    for path in physical_paths {
                        let _ = recycle_bin::delete_permanently(&path);
                    }
                    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                }
                FileOperationRequest::EmptyRecycleBin => {
                    let _ = recycle_bin::empty_recycle_bin();
                    let _ = result_sender.send(FileOperationResult::RecycleBinChanged);
                }
            }

            // Notify general completion for other operations
            let _ = result_sender.send(FileOperationResult::Finished);
        }

        unsafe {
            CoUninitialize();
        }
    });
}
