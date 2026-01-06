//! Native Windows Shell context menu extraction and invocation
//! Follows .cursorrules: single responsibility, < 300 lines

use std::path::Path;
use std::os::windows::ffi::OsStrExt;
use std::ffi::CStr;
use windows::{
    core::*, Win32::Foundation::*, Win32::System::Com::*, Win32::UI::Shell::{Common::ITEMIDLIST, *},
    Win32::UI::WindowsAndMessaging::*,
};
use crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba;

/// Represents a single item in the shell context menu
pub struct ShellMenuItem {
    pub id: u32, // Command ID from QueryContextMenu
    pub text: String,
    pub icon_rgba: Option<(Vec<u8>, u32, u32)>,
    pub sub_items: Vec<ShellMenuItem>,
    pub is_separator: bool,
    pub is_enabled: bool,
    /// Shell command verb (e.g., "copy", "delete", "openas") for filtering
    pub command_string: Option<String>,
}

/// Context holding the native objects alive
pub struct ShellMenuContext {
    pub items: Vec<ShellMenuItem>,
    pub context_menu: IContextMenu,
}

/// Known items that we handle internally - filter from shell menu (matches Files)
const KNOWN_VERBS: &[&str] = &[
    "opennew", "opencontaining", "opennewprocess",
    "runas", "runasuser", "pintohome", "PinToStartScreen",
    "cut", "copy", "paste", "delete", "properties", "link",
    "Windows.ModernShare", "Windows.Share", "setdesktopwallpaper",
    "eject", "rename", "explore", "openinfiles", "extract",
    "copyaspath", "undelete", "empty", "format", "rotate90", "rotate270",
];

/// Check if a verb should be filtered (handled by our UI)
pub fn is_known_verb(verb: &str) -> bool {
    KNOWN_VERBS.iter().any(|&v| v.eq_ignore_ascii_case(verb))
}

/// Extracts native shell menu items for a path
pub fn extract_shell_menu(hwnd: HWND, path: &Path) -> Result<ShellMenuContext> {
    unsafe {
        // Parse the path to a PIDL
        let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(wide_path.as_ptr()), None, &mut pidl, 0, None)?;

        if pidl.is_null() {
            return Err(Error::from_win32());
        }

        // Get parent folder and child PIDL
        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let parent_folder: IShellFolder = SHBindToParent(pidl, Some(&mut child))?;
        let items_ptr: [*const ITEMIDLIST; 1] = [child as *const ITEMIDLIST];

        // Get IContextMenu
        let context_menu: IContextMenu = parent_folder.GetUIObjectOf(hwnd, &items_ptr, None)?;
        
        // Create a temporary popup menu to extract items
        let hmenu = CreatePopupMenu()?;
        context_menu.QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL)?;

        let count = GetMenuItemCount(hmenu);
        let mut items = Vec::new();

        for i in 0..count {
            if let Some(item) = extract_item_info(&context_menu, hmenu, i as u32) {
                items.push(item);
            }
        }

        let _ = DestroyMenu(hmenu);
        CoTaskMemFree(Some(pidl as _));

        Ok(ShellMenuContext {
            items,
            context_menu,
        })
    }
}

/// Get command string (verb) for a menu item
unsafe fn get_command_string(context_menu: &IContextMenu, cmd_id: u32) -> Option<String> {
    // Avoid AccessViolationException on some items (NVIDIA, etc.)
    if cmd_id > 5000 {
        return None;
    }
    
    let mut buffer = [0u8; 256];
    let result = context_menu.GetCommandString(
        (cmd_id - 1) as usize,  // Offset by -1 as per QueryContextMenu
        GCS_VERBA,
        None,
        PSTR::from_raw(buffer.as_mut_ptr()),
        buffer.len() as u32,
    );
    
    if result.is_ok() {
        if let Ok(cstr) = CStr::from_bytes_until_nul(&buffer) {
            if let Ok(s) = cstr.to_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

unsafe fn extract_item_info(context_menu: &IContextMenu, hmenu: HMENU, index: u32) -> Option<ShellMenuItem> {
    let mut info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_BITMAP | MIIM_SUBMENU | MIIM_STRING,
        dwTypeData: PWSTR::null(),
        cch: 0,
        ..Default::default()
    };

    // First call to get string length
    if GetMenuItemInfoW(hmenu, index, true, &mut info).is_err() {
        return None;
    }

    let is_separator = (info.fType & MFT_SEPARATOR) != MENU_ITEM_TYPE(0);
    let is_enabled = (info.fState & MFS_DISABLED) == MENU_ITEM_STATE(0);
    
    let mut text = String::new();
    if !is_separator && info.cch > 0 {
        let mut buffer = vec![0u16; info.cch as usize + 1];
        info.dwTypeData = PWSTR(buffer.as_mut_ptr());
        info.cch += 1;
        let _ = GetMenuItemInfoW(hmenu, index, true, &mut info);
        text = String::from_utf16_lossy(&buffer)
            .trim_matches('\0')
            .replace('&', ""); // Remove keyboard mnemonics for egui
    }
    
    // Get command string (verb) for filtering
    let command_string = if !is_separator && info.wID >= 1 {
        get_command_string(context_menu, info.wID)
    } else {
        None
    };

    let icon_rgba = if !info.hbmpItem.0.is_null() && info.hbmpItem.0 != HBMMENU_CALLBACK.0 as *mut _ {
        hbitmap_to_rgba(info.hbmpItem).ok()
    } else {
        None
    };

    let mut sub_items = Vec::new();
    if !info.hSubMenu.0.is_null() {
        let sub_count = GetMenuItemCount(info.hSubMenu);
        for i in 0..sub_count {
            if let Some(sub_item) = extract_item_info(context_menu, info.hSubMenu, i as u32) {
                sub_items.push(sub_item);
            }
        }
    }

    Some(ShellMenuItem {
        id: info.wID,
        text,
        icon_rgba,
        sub_items,
        is_separator,
        is_enabled,
        command_string,
    })
}

pub fn invoke_menu_command(
    hwnd: HWND,
    context_menu: &IContextMenu,
    command_id: u32,
    screen_x: i32,
    screen_y: i32,
) -> Result<()> {
    unsafe {
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

        context_menu.InvokeCommand(std::mem::transmute(&invoke))
    }
}
