use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::*;

/// Helper to create a double-null-terminated wide string (required by SHFileOperation).
fn to_double_null_terminated(s: &str) -> Vec<u16> {
    s.encode_utf16()
        .chain(std::iter::once(0))
        .chain(std::iter::once(0))
        .collect()
}

/// Helper to create a double-null-terminated wide string buffer from multiple paths.
fn paths_to_double_null_terminated(paths: &[PathBuf]) -> Vec<u16> {
    let mut buffer = Vec::new();
    for path in paths {
        let path_str = path.to_string_lossy();
        buffer.extend(path_str.encode_utf16());
        buffer.push(0); // Null separator
    }
    buffer.push(0); // Double null terminator
    buffer
}

/// Deletes a file or directory using Windows Shell (moves to Recycle Bin by default).
/// Returns true if operation was successful (not cancelled).
pub fn delete_item_with_shell(path: &Path, hwnd: HWND) -> bool {
    let path_str = path.to_string_lossy();
    let from_vec = to_double_null_terminated(&path_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR::default(),
        fFlags: (FOF_ALLOWUNDO | FOF_WANTNUKEWARNING).0 as u16,
        ..Default::default()
    };

    // SAFETY: op is initialized with valid double-null terminated string.
    let result = unsafe { SHFileOperationW(&mut op) };

    // Result 0 means success. fAnyOperationsAborted is set if user cancelled.
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Deletes multiple files or directories using Windows Shell (moves to Recycle Bin by default).
/// Returns true if operation was successful (not cancelled).
pub fn delete_items_with_shell(paths: &[PathBuf], hwnd: HWND) -> bool {
    if paths.is_empty() {
        return false;
    }

    let from_vec = paths_to_double_null_terminated(paths);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR::default(),
        fFlags: (FOF_ALLOWUNDO | FOF_WANTNUKEWARNING).0 as u16,
        ..Default::default()
    };

    // SAFETY: op is initialized with valid double-null terminated string buffer.
    let result = unsafe { SHFileOperationW(&mut op) };

    // Result 0 means success. fAnyOperationsAborted is set if user cancelled.
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Renames a file or directory using Windows Shell.
/// Returns true if operation was successful.
pub fn rename_item_with_shell(path: &Path, new_name: &str, hwnd: HWND) -> bool {
    let parent = match path.parent() {
        Some(p) => p,
        None => return false,
    };

    let new_path = parent.join(new_name);

    // If destination exists, avoid merge/replace side-effects from FO_RENAME.
    if new_path.exists() {
        return false;
    }

    let from_str = path.to_string_lossy();
    let to_str = new_path.to_string_lossy();

    let from_vec = to_double_null_terminated(&from_str);
    let to_vec = to_double_null_terminated(&to_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_RENAME,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR(to_vec.as_ptr()),
        // Keep undo support and allow normal Windows error dialogs.
        fFlags: (FOF_ALLOWUNDO).0 as u16,
        ..Default::default()
    };

    // SAFETY: op is initialized with valid double-null terminated strings.
    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Copies multiple files/directories to a destination using a single Windows Shell operation.
/// This produces a single progress dialog for all files (batch behavior).
pub fn copy_items_with_shell(paths: &[PathBuf], dest_folder: &Path, hwnd: HWND) -> bool {
    if paths.is_empty() {
        return false;
    }
    if paths.len() == 1 {
        return copy_item_with_shell(&paths[0], dest_folder, hwnd);
    }

    let from_vec = paths_to_double_null_terminated(paths);
    let to_str = dest_folder.to_string_lossy();
    let to_vec = to_double_null_terminated(&to_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_COPY,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR(to_vec.as_ptr()),
        fFlags: (FOF_ALLOWUNDO).0 as u16,
        ..Default::default()
    };

    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Moves multiple files/directories to a destination using a single Windows Shell operation.
/// This produces a single progress dialog for all files (batch behavior).
pub fn move_items_with_shell(paths: &[PathBuf], dest_folder: &Path, hwnd: HWND) -> bool {
    if paths.is_empty() {
        return false;
    }
    if paths.len() == 1 {
        return move_item_with_shell(&paths[0], dest_folder, hwnd);
    }

    let from_vec = paths_to_double_null_terminated(paths);
    let to_str = dest_folder.to_string_lossy();
    let to_vec = to_double_null_terminated(&to_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_MOVE,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR(to_vec.as_ptr()),
        fFlags: (FOF_ALLOWUNDO).0 as u16,
        ..Default::default()
    };

    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Copies a file or directory using Windows Shell.
/// Returns true if operation was successful.
pub fn copy_item_with_shell(path: &Path, dest_folder: &Path, hwnd: HWND) -> bool {
    let from_str = path.to_string_lossy();
    let to_str = dest_folder.to_string_lossy();

    let from_vec = to_double_null_terminated(&from_str);
    let to_vec = to_double_null_terminated(&to_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_COPY,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR(to_vec.as_ptr()),
        fFlags: (FOF_ALLOWUNDO).0 as u16,
        ..Default::default()
    };

    // SAFETY: op is initialized with valid double-null terminated string.
    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Moves a file or directory using Windows Shell.
/// Returns true if operation was successful.
pub fn move_item_with_shell(path: &Path, dest_folder: &Path, hwnd: HWND) -> bool {
    // Skip if moving to same folder.
    if let Some(parent) = path.parent() {
        if parent == dest_folder {
            return false;
        }
    }

    let from_str = path.to_string_lossy();
    let to_str = dest_folder.to_string_lossy();

    let from_vec = to_double_null_terminated(&from_str);
    let to_vec = to_double_null_terminated(&to_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_MOVE,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR(to_vec.as_ptr()),
        fFlags: (FOF_ALLOWUNDO).0 as u16,
        ..Default::default()
    };

    // SAFETY: op is initialized with valid double-null terminated string.
    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}
