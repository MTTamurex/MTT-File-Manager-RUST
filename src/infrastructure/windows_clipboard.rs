//! Windows clipboard integration for file operations
//!
//! Uses clipboard-win crate to interact with Windows clipboard using CF_HDROP format.
//! This allows integration with native context menus and cross-application copy/paste.

use clipboard_win::{formats, Clipboard, Getter, Setter};
use std::path::PathBuf;
use windows::Win32::Foundation::HWND;

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

/// Result of publishing files to the Windows clipboard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipboardWriteResult {
    /// Both CF_HDROP and Preferred DropEffect were published.
    Complete,
    /// CF_HDROP was published, but the preferred operation was not.
    /// Windows Shell consumers safely treat this as Copy.
    FilesOnly,
}

/// Copies file paths to the Windows clipboard (CF_HDROP format)
///
/// # Arguments
/// * `paths` - Slice of file paths to copy
/// * `owner` - Application window that will own the clipboard data
///
/// # Returns
/// * `Ok(ClipboardWriteResult)` when CF_HDROP was published
/// * `Err(String)` with error message on failure
pub fn copy_files_to_clipboard(
    paths: &[PathBuf],
    owner: HWND,
) -> Result<ClipboardWriteResult, String> {
    write_files_to_clipboard(paths, owner, DROPEFFECT_COPY, "copy")
}

/// Cuts file paths to the Windows clipboard (CF_HDROP format with MOVE effect)
///
/// # Arguments
/// * `paths` - Slice of file paths to cut
/// * `owner` - Application window that will own the clipboard data
///
/// # Returns
/// * `Ok(ClipboardWriteResult)` when CF_HDROP was published
/// * `Err(String)` with error message on failure
pub fn cut_files_to_clipboard(
    paths: &[PathBuf],
    owner: HWND,
) -> Result<ClipboardWriteResult, String> {
    write_files_to_clipboard(paths, owner, DROPEFFECT_MOVE, "move")
}

/// Writes a native file payload and, when possible, its preferred Shell operation.
///
/// `CF_HDROP` is sufficient for Explorer and other Windows applications to paste files.
/// A failed `Preferred DropEffect` must not turn a successfully published file list into
/// a text-only clipboard entry; without that hint, Shell consumers safely default to Copy.
fn write_files_to_clipboard(
    paths: &[PathBuf],
    owner: HWND,
    preferred_drop_effect: u32,
    operation_name: &str,
) -> Result<ClipboardWriteResult, String> {
    if paths.is_empty() {
        return Err("No files to copy or move".to_string());
    }
    if owner.0.is_null() {
        return Err("No clipboard owner window available".to_string());
    }

    let file_list: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let _clip = Clipboard::new_attempts_for(owner.0, 10)
        .map_err(|e| format!("Failed to open clipboard: {:?}", e))?;

    // FileList writes with NoClear. Clear explicitly so a failed effect write cannot
    // leave a stale Copy/Move effect associated with the new file list.
    clipboard_win::empty().map_err(|e| format!("Failed to clear clipboard: {:?}", e))?;

    // Set files using CF_HDROP format via FileList
    formats::FileList
        .write_clipboard(&file_list)
        .map_err(|e| format!("Failed to write file list to clipboard: {:?}", e))?;

    if let Err(error) = set_preferred_drop_effect(preferred_drop_effect) {
        log::warn!(
            "[Clipboard] CF_HDROP was published, but Preferred DropEffect for {} failed; preserving the native file payload: {}",
            operation_name,
            error
        );
        return Ok(ClipboardWriteResult::FilesOnly);
    }

    Ok(ClipboardWriteResult::Complete)
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
