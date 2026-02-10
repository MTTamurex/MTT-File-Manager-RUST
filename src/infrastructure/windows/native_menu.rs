//! Native Windows Shell context menu extraction and invocation
//! Follows .cursorrules: single responsibility, < 300 lines

use crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba;
use std::ffi::CStr;
use std::os::windows::ffi::OsStrExt;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::Common::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

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
    /// For lazy-loaded submenus: stores the HMENU handle to load on demand
    /// This is Some() when sub_items is empty but a submenu exists
    pub pending_submenu_handle: Option<isize>,
    /// Index of this item in parent menu (for WM_INITMENUPOPUP)
    pub parent_index: u32,
}

/// Context holding the native objects alive
pub struct ShellMenuContext {
    pub items: std::cell::RefCell<Vec<ShellMenuItem>>,
    pub context_menu: IContextMenu,
    /// Keep the root menu handle alive for on-demand submenu loading
    hmenu: HMENU,
}

impl Drop for ShellMenuContext {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.hmenu);
        }
    }
}

/// Known items that we handle internally - filter from shell menu to avoid duplicates
const KNOWN_VERBS: &[&str] = &[
    "cut",
    "copy",
    "paste",
    "delete",
    "properties",
    "rename",
    "open",
    "explore",
    "opennew",
    "opencontaining",
    "pintohome",
    "rversions",
    "copyaspath",
    "link",
];

/// Check if a verb should be filtered (handled by our UI)
pub fn is_known_verb(verb: &str) -> bool {
    KNOWN_VERBS.iter().any(|&v| v.eq_ignore_ascii_case(verb))
}

/// Extracts native shell menu items for a path
/// Shell extensions may show fewer items on first call due to Windows lazy loading.
/// Call warmup_shell_extensions() on app startup to pre-initialize.
pub fn extract_shell_menu(hwnd: HWND, paths: &[std::path::PathBuf]) -> Result<ShellMenuContext> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CALL_COUNT: AtomicU32 = AtomicU32::new(0);
    let call_num = CALL_COUNT.fetch_add(1, Ordering::SeqCst) + 1;

    if paths.is_empty() {
        return Err(Error::from_win32());
    }

    unsafe {
        eprintln!(
            "[ShellMenu] ===== EXTRACTION #{} for: {:?} items =====",
            call_num,
            paths.len()
        );

        // Parse all paths to PIDLs and collect children
        let mut pidls_to_free = Vec::with_capacity(paths.len());
        let mut child_pidls = Vec::with_capacity(paths.len());
        let mut parent_folder_opt: Option<IShellFolder> = None;

        for path in paths {
            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(wide_path.as_ptr()), None, &mut pidl, 0, None).is_ok()
                && !pidl.is_null()
            {
                pidls_to_free.push(pidl);

                let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
                if let Ok(folder) = SHBindToParent(pidl, Some(&mut child)) {
                    if parent_folder_opt.is_none() {
                        parent_folder_opt = Some(folder);
                    }
                    child_pidls.push(child as *const ITEMIDLIST);
                }
            }
        }

        if parent_folder_opt.is_none() || child_pidls.is_empty() {
            for pidl in pidls_to_free {
                CoTaskMemFree(Some(pidl as _));
            }
            return Err(Error::from_win32());
        }

        let parent_folder = parent_folder_opt.unwrap();

        // Get IContextMenu
        let context_menu: IContextMenu = parent_folder.GetUIObjectOf(hwnd, &child_pidls, None)?;

        // Create popup menu to extract items
        let hmenu = CreatePopupMenu()?;
        context_menu
            .QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL)
            .ok()?;

        let count = GetMenuItemCount(Some(hmenu));
        eprintln!("[ShellMenu] Total menu items: {}", count);

        // Extract items
        let mut items = Vec::new();
        let mut pending_count = 0;
        for i in 0..count {
            if let Some(item) = extract_item_info(&context_menu, hmenu, i as u32, false) {
                if item.pending_submenu_handle.is_some() {
                    pending_count += 1;
                    eprintln!("[ShellMenu] Item '{}' has PENDING submenu", item.text);
                } else if !item.sub_items.is_empty() {
                    eprintln!(
                        "[ShellMenu] Item '{}' has {} sub-items",
                        item.text,
                        item.sub_items.len()
                    );
                }
                items.push(item);
            }
        }
        eprintln!(
            "[ShellMenu] Extracted {} items, {} with pending submenus",
            items.len(),
            pending_count
        );

        for pidl in pidls_to_free {
            CoTaskMemFree(Some(pidl as _));
        }

        Ok(ShellMenuContext {
            items: std::cell::RefCell::new(items),
            context_menu,
            hmenu,
        })
    }
}

/// Warmup function to pre-initialize shell extensions
/// Call this on app startup (e.g., with C:\ as path) to ensure
/// shell extensions like WinRAR, Send to, Include in library are loaded
pub fn warmup_shell_extensions(hwnd: HWND) {
    // Use the system drive root to trigger shell extension loading
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let warmup_path = std::path::PathBuf::from(format!("{}\\", system_drive));

    eprintln!(
        "[ShellMenu] Warming up shell extensions with {:?}...",
        warmup_path
    );

    if let Ok(_ctx) = extract_shell_menu(hwnd, &[warmup_path]) {
        eprintln!("[ShellMenu] Folder warmup complete");
    } else {
        eprintln!("[ShellMenu] Folder warmup failed");
    }

    // Use a temporary file to trigger file-level shell extensions (e.g., 7-Zip, WinRAR)
    let temp_file = std::env::temp_dir().join("mtt_warmup_dummy.txt");
    let _ = std::fs::File::create(&temp_file);
    if let Ok(_ctx) = extract_shell_menu(hwnd, std::slice::from_ref(&temp_file)) {
        eprintln!("[ShellMenu] File warmup complete");
    } else {
        eprintln!("[ShellMenu] File warmup failed");
    }
    let _ = std::fs::remove_file(&temp_file);

    eprintln!("[ShellMenu] Shell extensions initialized");
}

/// Get command string (verb) for a menu item
unsafe fn get_command_string(context_menu: &IContextMenu, cmd_id: u32) -> Option<String> {
    // Avoid AccessViolationException on some items (NVIDIA, etc.)
    if cmd_id > 5000 {
        return None;
    }

    let mut buffer = [0u8; 256];
    let result = context_menu.GetCommandString(
        (cmd_id - 1) as usize, // Offset by -1 as per QueryContextMenu
        GCS_VERBA,
        None,
        PSTR(buffer.as_mut_ptr()),
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

unsafe fn extract_item_info(
    context_menu: &IContextMenu,
    hmenu: HMENU,
    index: u32,
    recursive: bool,
) -> Option<ShellMenuItem> {
    let mut info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_BITMAP | MIIM_SUBMENU | MIIM_STRING,
        dwTypeData: PWSTR::default(),
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

    let icon_rgba =
        if !info.hbmpItem.0.is_null() && !std::ptr::eq(info.hbmpItem.0, HBMMENU_CALLBACK.0) {
            hbitmap_to_rgba(info.hbmpItem).ok()
        } else {
            None
        };

    let mut sub_items = Vec::new();
    let mut pending_submenu_handle = None;

    if !info.hSubMenu.0.is_null() {
        if recursive {
            // Send WM_INITMENUPOPUP BEFORE checking item count to trigger lazy loading
            if let Ok(ctx2) = context_menu.cast::<IContextMenu2>() {
                let _ = ctx2
                    .HandleMenuMsg(
                        WM_INITMENUPOPUP,
                        WPARAM(info.hSubMenu.0 as usize),
                        LPARAM(index as isize),
                    )
                    .ok();
            }

            let sub_count = GetMenuItemCount(Some(info.hSubMenu));
            if sub_count > 0 {
                // Submenu has items - extract them recursively (only if permitted)
                for i in 0..sub_count {
                    if let Some(sub_item) =
                        extract_item_info(context_menu, info.hSubMenu, i as u32, true)
                    {
                        sub_items.push(sub_item);
                    }
                }
            } else {
                pending_submenu_handle = Some(info.hSubMenu.0 as isize);
            }
        } else {
            // Lazy mode: Store handle for on-demand loading
            pending_submenu_handle = Some(info.hSubMenu.0 as isize);
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
        pending_submenu_handle,
        parent_index: index,
    })
}

impl ShellMenuContext {
    /// Load a pending submenu on demand (called when user hovers over the submenu in UI)
    /// This sends WM_INITMENUPOPUP to trigger lazy loading and extracts the items
    pub fn load_pending_submenu(&self, item: &mut ShellMenuItem) -> bool {
        if let Some(hmenu_ptr) = item.pending_submenu_handle.take() {
            unsafe {
                let hsubmenu = HMENU(hmenu_ptr as *mut _);

                // Send WM_INITMENUPOPUP to trigger lazy loading
                if let Ok(ctx2) = self.context_menu.cast::<IContextMenu2>() {
                    let _ = ctx2
                        .HandleMenuMsg(
                            WM_INITMENUPOPUP,
                            WPARAM(hmenu_ptr as usize),
                            LPARAM(item.parent_index as isize),
                        )
                        .ok();
                }

                // Now extract the items
                let sub_count = GetMenuItemCount(Some(hsubmenu));
                for i in 0..sub_count {
                    if let Some(sub_item) =
                        extract_item_info(&self.context_menu, hsubmenu, i as u32, false)
                    {
                        item.sub_items.push(sub_item);
                    }
                }

                return !item.sub_items.is_empty();
            }
        }
        false
    }

    /// Check if an item has a pending submenu that needs loading
    pub fn has_pending_submenu(item: &ShellMenuItem) -> bool {
        item.pending_submenu_handle.is_some()
    }
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
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: POINT {
                x: screen_x,
                y: screen_y,
            },
            ..Default::default()
        };

        context_menu.InvokeCommand(std::ptr::addr_of!(invoke) as *const _)
    }
}

pub fn show_properties_dialog(hwnd: HWND, path: &std::path::Path) -> Result<()> {
    use windows::Win32::UI::Shell::{SHObjectProperties, SHOP_FILEPATH};

    let path_str = path.to_string_lossy();
    let wide_path: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        // SAFETY: wide_path is null-terminated, SHOP_FILEPATH specifies we are passing a path string
        SHObjectProperties(Some(hwnd), SHOP_FILEPATH, PCWSTR(wide_path.as_ptr()), None).ok()
    }
}
