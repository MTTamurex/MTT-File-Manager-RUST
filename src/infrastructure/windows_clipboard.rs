//! Windows clipboard integration for file operations
//!
//! Uses clipboard-win crate to interact with Windows clipboard using CF_HDROP format.
//! This allows integration with native context menus and cross-application copy/paste.

use clipboard_win::{formats, Clipboard, Getter, Setter};
use std::path::PathBuf;

/// Preferred drop effect values (from shlobj.h)
/// Used to indicate if the clipboard operation is a Copy or Move (Cut)
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

/// Represents the type of clipboard operation
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipboardFileOp {
    Copy,
    Move,
}

/// Copies file paths to the Windows clipboard (CF_HDROP format)
///
/// # Arguments
/// * `paths` - Slice of file paths to copy
///
/// # Returns
/// * `Ok(())` on success
/// * `Err(String)` with error message on failure
pub fn copy_files_to_clipboard(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No files to copy".to_string());
    }

    // Convert paths to the format expected by clipboard
    let file_list: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let _clip =
        Clipboard::new_attempts(10).map_err(|e| format!("Failed to open clipboard: {:?}", e))?;

    // Set files using CF_HDROP format via FileList
    formats::FileList
        .write_clipboard(&file_list)
        .map_err(|e| format!("Failed to write file list to clipboard: {:?}", e))?;

    // Set preferred drop effect to COPY
    set_preferred_drop_effect(DROPEFFECT_COPY)?;

    Ok(())
}

/// Cuts file paths to the Windows clipboard (CF_HDROP format with MOVE effect)
///
/// # Arguments
/// * `paths` - Slice of file paths to cut
///
/// # Returns
/// * `Ok(())` on success
/// * `Err(String)` with error message on failure
pub fn cut_files_to_clipboard(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("No files to cut".to_string());
    }

    // Convert paths to the format expected by clipboard
    let file_list: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let _clip =
        Clipboard::new_attempts(10).map_err(|e| format!("Failed to open clipboard: {:?}", e))?;

    // Set files using CF_HDROP format via FileList
    formats::FileList
        .write_clipboard(&file_list)
        .map_err(|e| format!("Failed to write file list to clipboard: {:?}", e))?;

    // Set preferred drop effect to MOVE
    set_preferred_drop_effect(DROPEFFECT_MOVE)?;

    Ok(())
}

/// Gets file paths from the Windows clipboard (CF_HDROP format)
///
/// # Returns
/// * `Some(Vec<PathBuf>)` if files are in clipboard
/// * `None` if clipboard doesn't contain files
pub fn get_files_from_clipboard() -> Option<Vec<PathBuf>> {
    let _clip = Clipboard::new_attempts(10).ok()?;

    let mut file_list: Vec<String> = Vec::new();
    if formats::FileList.read_clipboard(&mut file_list).is_ok() && !file_list.is_empty() {
        Some(file_list.into_iter().map(PathBuf::from).collect())
    } else {
        None
    }
}

/// Gets the clipboard operation type (Copy or Move)
///
/// # Returns
/// * `Some(ClipboardFileOp)` if drop effect is set
/// * `None` if not set (defaults to Copy behavior)
pub fn get_clipboard_operation() -> Option<ClipboardFileOp> {
    let effect = get_preferred_drop_effect().unwrap_or(DROPEFFECT_COPY);

    if effect & DROPEFFECT_MOVE != 0 {
        Some(ClipboardFileOp::Move)
    } else {
        Some(ClipboardFileOp::Copy) // Default to copy
    }
}

/// Checks if the clipboard contains files
pub fn has_files_in_clipboard() -> bool {
    get_files_from_clipboard().is_some_and(|files| !files.is_empty())
}

/// Gets the current Windows clipboard sequence number.
///
/// This increments whenever clipboard content changes and is used to detect
/// when internal file clipboard fallback becomes stale (e.g. user copied text).
pub fn clipboard_sequence_number() -> Option<u32> {
    use windows::Win32::System::DataExchange::GetClipboardSequenceNumber;

    let seq = unsafe { GetClipboardSequenceNumber() };
    if seq == 0 {
        None
    } else {
        Some(seq)
    }
}

// --- Internal helper functions ---

/// Sets the preferred drop effect in the clipboard
fn set_preferred_drop_effect(effect: u32) -> Result<(), String> {
    use windows::core::w;
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
    use windows::Win32::System::DataExchange::SetClipboardData;
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    unsafe {
        // Register the "Preferred DropEffect" format
        let format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        if format == 0 {
            return Err("Failed to register clipboard format".to_string());
        }

        // Allocate global memory for the effect value
        let hmem = GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of::<u32>())
            .map_err(|e| format!("GlobalAlloc failed: {:?}", e))?;

        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            let _ = GlobalFree(Some(hmem));
            return Err("GlobalLock failed".to_string());
        }

        // Write the effect value
        *(ptr as *mut u32) = effect;

        let _ = GlobalUnlock(hmem);

        // Set the clipboard data
        if let Err(e) = SetClipboardData(format, Some(windows::Win32::Foundation::HANDLE(hmem.0))) {
            let _ = GlobalFree(Some(hmem));
            return Err(format!("SetClipboardData failed: {:?}", e));
        }
    }

    Ok(())
}

/// Gets the preferred drop effect from the clipboard
fn get_preferred_drop_effect() -> Option<u32> {
    use windows::core::w;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};

    unsafe {
        // MUST open clipboard before GetClipboardData
        if OpenClipboard(None).is_err() {
            return None;
        }

        let format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        if format == 0 {
            let _ = CloseClipboard();
            return None;
        }

        let handle = match GetClipboardData(format) {
            Ok(h) => h,
            Err(_) => {
                let _ = CloseClipboard();
                return None;
            }
        };

        if handle.is_invalid() {
            let _ = CloseClipboard();
            return None;
        }

        let hmem = windows::Win32::Foundation::HGLOBAL(handle.0);
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            let _ = CloseClipboard();
            return None;
        }

        let effect = *(ptr as *const u32);
        let _ = GlobalUnlock(hmem);
        let _ = CloseClipboard();

        Some(effect)
    }
}
