//! Worker thread for Windows Shell file operations.
//! Ensures COM is initialized as STA (COINIT_APARTMENTTHREADED) for correct shell behavior.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

use crate::infrastructure::windows::shell_operations;
use crate::infrastructure::windows::recycle_bin;

/// Transparent wrapper for HWND to make it Send.
/// SAFETY: HWNDs are globally valid in Windows and can be used from any thread.
#[derive(Clone, Copy)]
pub struct SendHwnd(pub HWND);
unsafe impl Send for SendHwnd {}

/// Requests that can be sent to the file operation worker.
pub enum FileOperationRequest {
    Delete {
        path: PathBuf,
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
        physical_path: PathBuf,
        original_path: PathBuf,
    },
    DeletePermanently {
        physical_path: PathBuf,
    },
    EmptyRecycleBin,
}

impl FileOperationRequest {
    // Helper to wrap HWND
    pub fn delete(path: PathBuf, hwnd: HWND) -> Self {
        Self::Delete { path, hwnd: SendHwnd(hwnd) }
    }
    pub fn rename(path: PathBuf, new_name: String, hwnd: HWND) -> Self {
        Self::Rename { path, new_name, hwnd: SendHwnd(hwnd) }
    }
    pub fn copy(path: PathBuf, dest_folder: PathBuf, hwnd: HWND) -> Self {
        Self::Copy { path, dest_folder, hwnd: SendHwnd(hwnd) }
    }
    pub fn file_move(path: PathBuf, dest_folder: PathBuf, hwnd: HWND) -> Self {
        Self::Move { path, dest_folder, hwnd: SendHwnd(hwnd) }
    }
}

/// Starts the file operation worker thread.
pub fn start_file_operation_worker(receiver: Receiver<FileOperationRequest>) {
    std::thread::spawn(move || {
        // Initialize COM as Single-Threaded Apartment (STA)
        // This is critical for shell progress dialogs and proper COM behavior.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        while let Ok(request) = receiver.recv() {
            match request {
                FileOperationRequest::Delete { path, hwnd } => {
                    let _ = shell_operations::delete_item_with_shell(&path, hwnd.0);
                }
                FileOperationRequest::Rename { path, new_name, hwnd } => {
                    let _ = shell_operations::rename_item_with_shell(&path, &new_name, hwnd.0);
                }
                FileOperationRequest::Copy { path, dest_folder, hwnd } => {
                    let _ = shell_operations::copy_item_with_shell(&path, &dest_folder, hwnd.0);
                }
                FileOperationRequest::Move { path, dest_folder, hwnd } => {
                    let _ = shell_operations::move_item_with_shell(&path, &dest_folder, hwnd.0);
                }
                FileOperationRequest::RestoreFromRecycleBin { physical_path, original_path } => {
                    let _ = recycle_bin::restore_from_recycle_bin(&physical_path, &original_path);
                }
                FileOperationRequest::DeletePermanently { physical_path } => {
                    let _ = recycle_bin::delete_permanently(&physical_path);
                }
                FileOperationRequest::EmptyRecycleBin => {
                    let _ = recycle_bin::empty_recycle_bin();
                }
            }
        }

        unsafe {
            CoUninitialize();
        }
    });
}
