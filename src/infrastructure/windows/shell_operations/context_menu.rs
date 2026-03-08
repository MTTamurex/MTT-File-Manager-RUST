use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::Common::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Opens a file with its default application using ShellExecuteW.
pub fn open_with_shell(path: &Path) -> Result<()> {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

        let hinst = ShellExecuteW(
            None,
            PCWSTR::default(),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR::default(),
            PCWSTR::default(),
            SW_SHOW,
        );

        let code = hinst.0 as isize;
        if code <= 32 {
            return Err(Error::new(
                E_FAIL,
                format!("ShellExecuteW failed with code {} for path {:?}", code, path),
            ));
        }

        Ok(())
    }
}

/// RAII guard to balance CoInitializeEx/CoUninitialize calls.
struct ComGuard;

impl ComGuard {
    fn new() -> Result<Option<Self>> {
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

/// Result of showing a shell context menu.
#[derive(Debug)]
pub struct ContextMenuResult {
    /// True if the menu was cancelled (dismissed without selection).
    pub was_cancelled: bool,
    /// Cursor position when the menu was dismissed (screen coordinates).
    pub cursor_x: i32,
    pub cursor_y: i32,
    /// True if right button is currently pressed (for right-click detection).
    pub right_button_down: bool,
}

/// Shows the native Windows shell context menu for a single filesystem path.
/// Returns Ok with info about how the menu was dismissed.
pub fn show_shell_context_menu(
    hwnd: HWND,
    path: &Path,
    screen_x: i32,
    screen_y: i32,
) -> Result<ContextMenuResult> {
    let _com_guard = ComGuard::new()?;

    // Convert path to wide string for SHParseDisplayName.
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // RAII guard to ensure PIDL is always freed, even on early `?` returns.
    struct PidlGuard(*mut ITEMIDLIST);
    impl Drop for PidlGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CoTaskMemFree(Some(self.0 as _)); }
            }
        }
    }

    // RAII guard to ensure HMENU is always destroyed, even on early `?` returns.
    struct MenuGuard(HMENU);
    impl Drop for MenuGuard {
        fn drop(&mut self) {
            if !self.0.0.is_null() {
                unsafe { let _ = DestroyMenu(self.0); }
            }
        }
    }

    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(wide_path.as_ptr()), None, &mut pidl, 0, None)?;

        let pidl_guard = PidlGuard(pidl);

        if pidl_guard.0.is_null() {
            return Ok(ContextMenuResult {
                was_cancelled: true,
                cursor_x: screen_x,
                cursor_y: screen_y,
                right_button_down: false,
            });
        }

        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let parent_folder: IShellFolder = SHBindToParent(pidl_guard.0, Some(&mut child))?;

        let items: [*const ITEMIDLIST; 1] = [child as *const ITEMIDLIST];
        let context_menu: IContextMenu = parent_folder.GetUIObjectOf(hwnd, &items, None)?;

        let hmenu = CreatePopupMenu()?;
        let menu_guard = MenuGuard(hmenu);

        if menu_guard.0.0.is_null() {
            return Ok(ContextMenuResult {
                was_cancelled: true,
                cursor_x: screen_x,
                cursor_y: screen_y,
                right_button_down: false,
            });
        }

        context_menu
            .QueryContextMenu(menu_guard.0, 0, 1, 0x7FFF, windows::Win32::UI::Shell::CMF_NORMAL)
            .ok()?;

        let command_id = TrackPopupMenuEx(
            menu_guard.0,
            (TPM_RETURNCMD | TPM_RIGHTBUTTON).0,
            screen_x,
            screen_y,
            hwnd,
            None,
        )
        .0 as u32;

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let right_down = windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x02) < 0;

        let was_cancelled = command_id == 0;

        if command_id != 0 {
            let invoke = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: CMIC_MASK_PTINVOKE,
                hwnd,
                lpVerb: PCSTR((command_id - 1) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0,
                ptInvoke: POINT {
                    x: screen_x,
                    y: screen_y,
                },
                ..Default::default()
            };

            context_menu.InvokeCommand(
                &invoke as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO,
            )?;
        }

        // pidl_guard and menu_guard are dropped here, cleaning up PIDL and HMENU
        // regardless of which path we took.

        Ok(ContextMenuResult {
            was_cancelled,
            cursor_x: cursor.x,
            cursor_y: cursor.y,
            right_button_down: right_down,
        })
    }
}
