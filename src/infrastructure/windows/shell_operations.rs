//! Windows shell operations
//! Follows .cursorrules: single responsibility, < 300 lines

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::System::Com::*,
    Win32::UI::Input::KeyboardAndMouse::*,
    Win32::UI::Shell::Common::*,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
};

/// Opens a file with its default application using ShellExecuteW.
pub fn open_with_shell(path: &Path) {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let _ = ShellExecuteW(
            None,
            PCWSTR(std::ptr::null()),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR(std::ptr::null()),
            PCWSTR(std::ptr::null()),
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
        unsafe { CoUninitialize(); }
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
pub fn show_shell_context_menu(hwnd: HWND, path: &Path, screen_x: i32, screen_y: i32) -> windows::core::Result<ContextMenuResult> {
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
        context_menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL)?;

        let command_id = TrackPopupMenuEx(
            hmenu,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            screen_x,
            screen_y,
            hwnd,
            None,
        ).0 as u32;

        // Get cursor position after menu closes
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        
        // Check if any mouse button is pressed
        let right_down = GetAsyncKeyState(0x02) < 0; // VK_RBUTTON = 0x02

        let was_cancelled = command_id == 0;

        if command_id != 0 {
            let invoke = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: CMIC_MASK_PTINVOKE,
                hwnd,
                lpVerb: PCSTR((command_id - 1) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0 as i32,
                ptInvoke: POINT { x: screen_x, y: screen_y },
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
