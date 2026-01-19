//! Windows shell operations
//! Follows .cursorrules: single responsibility, < 300 lines

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{
    core::*, Win32::Foundation::*, Win32::System::Com::*,
    Win32::UI::Shell::Common::*, Win32::UI::Shell::*, Win32::UI::WindowsAndMessaging::*,
};

/// Opens a file with its default application using ShellExecuteW.
pub fn open_with_shell(path: &Path) {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let _ = ShellExecuteW(
            None,
            PCWSTR::default(),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR::default(),
            PCWSTR::default(),
            SW_SHOW,
        );
    }
}

/// RAII guard to balance CoInitializeEx/CoUninitialize calls.
struct ComGuard;

impl ComGuard {
    fn new() -> windows::core::Result<Option<Self>> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(None);
        }
        if hr.is_err() {
            return Err(hr.into());
        }
        Ok(Some(Self))
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

/// Result of showing a shell context menu
#[derive(Debug)]
pub struct ContextMenuResult {
    /// True if the menu was cancelled (dismissed without selection)
    pub was_cancelled: bool,
    /// Cursor position when the menu was dismissed (screen coordinates)
    pub cursor_x: i32,
    pub cursor_y: i32,
    /// True if right button is currently pressed (for right-click detection)
    pub right_button_down: bool,
}

/// Shows the native Windows shell context menu for a single filesystem path at the given screen coordinates.
/// Returns Ok with info about how the menu was dismissed.
pub fn show_shell_context_menu(
    hwnd: HWND,
    path: &Path,
    screen_x: i32,
    screen_y: i32,
) -> windows::core::Result<ContextMenuResult> {
    let _com_guard = ComGuard::new()?;

    // Convert path to wide string for SHParseDisplayName
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        // SAFETY: wide_path is null-terminated and remains alive for the duration of the call.
        SHParseDisplayName(PCWSTR(wide_path.as_ptr()), None, &mut pidl, 0, None)?;

        if pidl.is_null() {
            return Ok(ContextMenuResult {
                was_cancelled: true,
                cursor_x: screen_x,
                cursor_y: screen_y,
                right_button_down: false,
            });
        }

        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        // SAFETY: pidl is valid and owned; SHBindToParent returns the parent folder and child PIDL.
        let parent_folder: IShellFolder = SHBindToParent(pidl, Some(&mut child))?;

        let items: [*const ITEMIDLIST; 1] = [child as *const ITEMIDLIST];

        // SAFETY: child references the item within parent_folder; hwnd is our window handle.
        let context_menu: IContextMenu = parent_folder.GetUIObjectOf(hwnd, &items, None)?;

        let hmenu = CreatePopupMenu()?;
        if hmenu.0.is_null() {
            CoTaskMemFree(Some(pidl as _));
            return Ok(ContextMenuResult {
                was_cancelled: true,
                cursor_x: screen_x,
                cursor_y: screen_y,
                right_button_down: false,
            });
        }

        // SAFETY: hmenu is a valid menu handle; command ids start at 1.
        context_menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, windows::Win32::UI::Shell::CMF_NORMAL).ok()?;

        let command_id = TrackPopupMenuEx(
            hmenu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            screen_x,
            screen_y,
            hwnd,
            None,
        )
        .0 as u32;

        // Get cursor position after menu closes
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        // Check if any mouse button is pressed
        let right_down = windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x02) < 0; // VK_RBUTTON = 0x02

        let was_cancelled = command_id == 0;

        if command_id != 0 {
            let invoke = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: CMIC_MASK_PTINVOKE,
                hwnd,
                lpVerb: PCSTR((command_id - 1) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0 as i32,
                ptInvoke: POINT {
                    x: screen_x,
                    y: screen_y,
                },
                ..Default::default()
            };

            // SAFETY: invoke contains valid fields; lpVerb uses command offset from QueryContextMenu base.
            context_menu.InvokeCommand(std::mem::transmute(&invoke))?;
        }

        DestroyMenu(hmenu)?;
        CoTaskMemFree(Some(pidl as _));

        Ok(ContextMenuResult {
            was_cancelled,
            cursor_x: cursor.x,
            cursor_y: cursor.y,
            right_button_down: right_down,
        })
    }
}
/// Helper to create a double-null-terminated wide string (required by SHFileOperation)
fn to_double_null_terminated(s: &str) -> Vec<u16> {
    s.encode_utf16()
        .chain(std::iter::once(0))
        .chain(std::iter::once(0))
        .collect()
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

    // SAFETY: op is initialized with valid double-null terminated string
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

    let from_str = path.to_string_lossy();
    let to_str = new_path.to_string_lossy();

    let from_vec = to_double_null_terminated(&from_str);
    let to_vec = to_double_null_terminated(&to_str);

    let mut op = SHFILEOPSTRUCTW {
        hwnd,
        wFunc: FO_RENAME,
        pFrom: PCWSTR(from_vec.as_ptr()),
        pTo: PCWSTR(to_vec.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NO_UI).0 as u16,
        ..Default::default()
    };

    // SAFETY: op is initialized with valid double-null terminated strings
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

    // SAFETY: op is initialized with valid double-null terminated string
    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Moves a file or directory using Windows Shell.
/// Returns true if operation was successful.
pub fn move_item_with_shell(path: &Path, dest_folder: &Path, hwnd: HWND) -> bool {
    // Skip if moving to same folder
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

    // SAFETY: op is initialized with valid double-null terminated string
    let result = unsafe { SHFileOperationW(&mut op) };
    result == 0 && op.fAnyOperationsAborted.0 == 0
}

/// Robust Copy using IFileOperation (supports virtual paths like ZIP items)
pub fn copy_item_with_file_op(path: &Path, dest_folder: &Path, hwnd: HWND) -> bool {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        
        // Use IFileOperation for modern Shell features (like ZIP extraction)
        let file_op: IFileOperation = match CoCreateInstance(&FileOperation, None, CLSCTX_ALL) {
            Ok(op) => op,
            Err(_) => return copy_item_with_shell(path, dest_folder, hwnd), 
        };
        
        let _ = file_op.SetOwnerWindow(hwnd);
        let _ = file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_WANTNUKEWARNING);
        
        let src_wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let src_item: IShellItem = match SHCreateItemFromParsingName(PCWSTR(src_wide.as_ptr()), None) {
            Ok(i) => i,
            Err(_) => return copy_item_with_shell(path, dest_folder, hwnd),
        };
        
        let dest_wide: Vec<u16> = dest_folder.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let dest_item: IShellItem = match SHCreateItemFromParsingName(PCWSTR(dest_wide.as_ptr()), None) {
            Ok(i) => i,
            Err(_) => return copy_item_with_shell(path, dest_folder, hwnd),
        };
        
        if file_op.CopyItem(&src_item, &dest_item, None, None).is_err() {
            return copy_item_with_shell(path, dest_folder, hwnd);
        }
        
        file_op.PerformOperations().is_ok()
    }
}
