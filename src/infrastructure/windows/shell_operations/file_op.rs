use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;

use super::shfile_ops::{
    copy_item_with_shell, copy_items_with_shell, move_item_with_shell, move_items_with_shell,
};

/// RAII guard for balanced COM initialization in file operations.
/// Previously, each function called CoInitializeEx without CoUninitialize,
/// leaking COM refcounts and kernel resources on every copy/move operation.
struct FileOpComGuard(bool);
impl FileOpComGuard {
    fn init() -> Self {
        let ok = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        Self(ok)
    }
}
impl Drop for FileOpComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

/// Robust copy using IFileOperation (supports virtual paths like ZIP items).
pub fn copy_item_with_file_op(path: &Path, dest_folder: &Path, hwnd: HWND) -> bool {
    let _com = FileOpComGuard::init();
    unsafe {
        // Use IFileOperation for modern Shell features (like ZIP extraction).
        let file_op: IFileOperation = match CoCreateInstance(&FileOperation, None, CLSCTX_ALL) {
            Ok(op) => op,
            Err(_) => return copy_item_with_shell(path, dest_folder, hwnd),
        };

        let _ = file_op.SetOwnerWindow(hwnd);
        let _ = file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_WANTNUKEWARNING);

        let src_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let src_item: IShellItem =
            match SHCreateItemFromParsingName(PCWSTR(src_wide.as_ptr()), None) {
                Ok(i) => i,
                Err(_) => return copy_item_with_shell(path, dest_folder, hwnd),
            };

        let dest_wide: Vec<u16> = dest_folder
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dest_item: IShellItem =
            match SHCreateItemFromParsingName(PCWSTR(dest_wide.as_ptr()), None) {
                Ok(i) => i,
                Err(_) => return copy_item_with_shell(path, dest_folder, hwnd),
            };

        if file_op.CopyItem(&src_item, &dest_item, None, None).is_err() {
            return copy_item_with_shell(path, dest_folder, hwnd);
        }

        file_op.PerformOperations().is_ok()
    }
}

/// Robust copy of multiple items using IFileOperation (supports virtual paths like ZIP items).
/// This produces a single progress dialog for all files.
pub fn copy_items_with_file_op(paths: &[PathBuf], dest_folder: &Path, hwnd: HWND) -> bool {
    if paths.is_empty() {
        return false;
    }

    let _com = FileOpComGuard::init();
    unsafe {
        // Use IFileOperation for modern Shell features (like ZIP extraction).
        let file_op: IFileOperation = match CoCreateInstance(&FileOperation, None, CLSCTX_ALL) {
            Ok(op) => op,
            Err(_) => return copy_items_with_shell(paths, dest_folder, hwnd),
        };

        let _ = file_op.SetOwnerWindow(hwnd);
        let _ = file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_WANTNUKEWARNING);

        // Get destination as IShellItem.
        let dest_wide: Vec<u16> = dest_folder
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dest_item: IShellItem =
            match SHCreateItemFromParsingName(PCWSTR(dest_wide.as_ptr()), None) {
                Ok(i) => i,
                Err(_) => return copy_items_with_shell(paths, dest_folder, hwnd),
            };

        // Add each source item to the operation.
        for path in paths {
            let src_wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let src_item: IShellItem =
                match SHCreateItemFromParsingName(PCWSTR(src_wide.as_ptr()), None) {
                    Ok(i) => i,
                    Err(_) => {
                        // Fallback to shell copy for this item.
                        if !copy_item_with_shell(path, dest_folder, hwnd) {
                            return false;
                        }
                        continue;
                    }
                };

            if file_op.CopyItem(&src_item, &dest_item, None, None).is_err() {
                return false;
            }
        }

        file_op.PerformOperations().is_ok()
    }
}

/// Robust move using IFileOperation (supports virtual paths like ZIP items).
pub fn move_item_with_file_op(path: &Path, dest_folder: &Path, hwnd: HWND) -> bool {
    // Skip if moving to same folder.
    if let Some(parent) = path.parent() {
        if parent == dest_folder {
            return false;
        }
    }

    let _com = FileOpComGuard::init();
    unsafe {
        // Use IFileOperation for modern Shell features.
        let file_op: IFileOperation = match CoCreateInstance(&FileOperation, None, CLSCTX_ALL) {
            Ok(op) => op,
            Err(_) => return move_item_with_shell(path, dest_folder, hwnd),
        };

        let _ = file_op.SetOwnerWindow(hwnd);
        let _ = file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_WANTNUKEWARNING);

        let src_wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let src_item: IShellItem =
            match SHCreateItemFromParsingName(PCWSTR(src_wide.as_ptr()), None) {
                Ok(i) => i,
                Err(_) => return move_item_with_shell(path, dest_folder, hwnd),
            };

        let dest_wide: Vec<u16> = dest_folder
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dest_item: IShellItem =
            match SHCreateItemFromParsingName(PCWSTR(dest_wide.as_ptr()), None) {
                Ok(i) => i,
                Err(_) => return move_item_with_shell(path, dest_folder, hwnd),
            };

        if file_op.MoveItem(&src_item, &dest_item, None, None).is_err() {
            return move_item_with_shell(path, dest_folder, hwnd);
        }

        file_op.PerformOperations().is_ok()
    }
}

/// Robust move of multiple items using IFileOperation (supports virtual paths like ZIP items).
pub fn move_items_with_file_op(paths: &[PathBuf], dest_folder: &Path, hwnd: HWND) -> bool {
    if paths.is_empty() {
        return false;
    }

    let _com = FileOpComGuard::init();
    unsafe {
        // Use IFileOperation for modern Shell features.
        let file_op: IFileOperation = match CoCreateInstance(&FileOperation, None, CLSCTX_ALL) {
            Ok(op) => op,
            Err(_) => return move_items_with_shell(paths, dest_folder, hwnd),
        };

        let _ = file_op.SetOwnerWindow(hwnd);
        let _ = file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_WANTNUKEWARNING);

        // Get destination as IShellItem.
        let dest_wide: Vec<u16> = dest_folder
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dest_item: IShellItem =
            match SHCreateItemFromParsingName(PCWSTR(dest_wide.as_ptr()), None) {
                Ok(i) => i,
                Err(_) => return move_items_with_shell(paths, dest_folder, hwnd),
            };

        // Add each source item to the operation.
        for path in paths {
            // Skip if moving to same folder.
            if let Some(parent) = path.parent() {
                if parent == dest_folder {
                    continue;
                }
            }

            let src_wide: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let src_item: IShellItem =
                match SHCreateItemFromParsingName(PCWSTR(src_wide.as_ptr()), None) {
                    Ok(i) => i,
                    Err(_) => {
                        // Fallback to shell move for this item.
                        if !move_item_with_shell(path, dest_folder, hwnd) {
                            return false;
                        }
                        continue;
                    }
                };

            if file_op.MoveItem(&src_item, &dest_item, None, None).is_err() {
                return false;
            }
        }

        file_op.PerformOperations().is_ok()
    }
}
