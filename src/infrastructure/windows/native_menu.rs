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

struct PidlCleanupGuard {
    pidls: Vec<*mut ITEMIDLIST>,
}

impl PidlCleanupGuard {
    fn new(capacity: usize) -> Self {
        Self {
            pidls: Vec::with_capacity(capacity),
        }
    }

    fn push(&mut self, pidl: *mut ITEMIDLIST) {
        self.pidls.push(pidl);
    }
}

impl Drop for PidlCleanupGuard {
    fn drop(&mut self) {
        unsafe {
            for pidl in self.pidls.drain(..) {
                CoTaskMemFree(Some(pidl as _));
            }
        }
    }
}

impl Drop for ShellMenuContext {
    fn drop(&mut self) {
        unsafe {
            // Bitmap handles exposed by shell extensions remain extension-owned.
            // The host owns only the HMENU it created.
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

pub fn is_filtered_shell_text(text: &str) -> bool {
    const FILTERED_TEXT: &[&str] = &[
        "pin to quick access",
        "fixar no acesso rápido",
        "restore previous versions",
        "restaurar versões anteriores",
        "copy as path",
        "copiar como caminho",
        "create shortcut",
        "criar atalho",
        "always keep on this device",
        "sempre manter neste dispositivo",
        "free up space",
        "liberar espaço",
        "open in terminal",
        "abrir no terminal",
        "open in terminal (admin)",
        "abrir no terminal (admin)",
    ];
    let lower = text.to_lowercase();
    FILTERED_TEXT.iter().any(|entry| lower.contains(entry))
}

fn selection_has_single_parent(paths: &[std::path::PathBuf]) -> bool {
    let Some(first_parent) = paths.first().and_then(|path| path.parent()) else {
        return paths.len() == 1;
    };
    paths.iter().skip(1).all(|path| {
        path.parent().is_some_and(|parent| {
            first_parent
                .to_string_lossy()
                .eq_ignore_ascii_case(&parent.to_string_lossy())
        })
    })
}

/// Extracts native shell menu items for a path
pub fn extract_shell_menu(hwnd: HWND, paths: &[std::path::PathBuf]) -> Result<ShellMenuContext> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CALL_COUNT: AtomicU32 = AtomicU32::new(0);
    let call_num = CALL_COUNT.fetch_add(1, Ordering::SeqCst) + 1;

    if paths.is_empty() {
        return Err(Error::from_thread());
    }
    if !selection_has_single_parent(paths) {
        return Err(Error::new(
            E_INVALIDARG,
            "Shell context menus require items from one parent folder",
        ));
    }

    unsafe {
        log::debug!(
            "[ShellMenu] ===== EXTRACTION #{} for: {:?} items =====",
            call_num,
            paths.len()
        );

        // Parse all paths to PIDLs and collect children
        let mut pidls_to_free = PidlCleanupGuard::new(paths.len().saturating_mul(2));
        let mut child_pidls = Vec::with_capacity(paths.len());
        let mut parent_folder_opt: Option<IShellFolder> = None;
        let mut expected_parent_pidl: Option<*mut ITEMIDLIST> = None;

        for path in paths {
            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(wide_path.as_ptr()), None, &mut pidl, 0, None)?;
            if pidl.is_null() {
                return Err(Error::new(
                    E_INVALIDARG,
                    "Shell returned a null item ID list",
                ));
            }
            pidls_to_free.push(pidl);

            let parent_pidl = ILClone(pidl);
            if parent_pidl.is_null() {
                return Err(Error::new(
                    E_OUTOFMEMORY,
                    "Failed to clone the Shell parent item ID list",
                ));
            }
            pidls_to_free.push(parent_pidl);
            if !ILRemoveLastID(Some(parent_pidl)).as_bool() {
                return Err(Error::new(
                    E_INVALIDARG,
                    "Shell item does not have a resolvable parent",
                ));
            }
            if let Some(expected) = expected_parent_pidl {
                if !ILIsEqual(expected, parent_pidl).as_bool() {
                    return Err(Error::new(
                        E_INVALIDARG,
                        "Shell context menu items resolved to different parent folders",
                    ));
                }
            } else {
                expected_parent_pidl = Some(parent_pidl);
            }

            let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
            let folder = SHBindToParent(pidl, Some(&mut child))?;
            if child.is_null() {
                return Err(Error::new(E_INVALIDARG, "Shell returned a null child item"));
            }
            if parent_folder_opt.is_none() {
                parent_folder_opt = Some(folder);
            }
            child_pidls.push(child as *const ITEMIDLIST);
        }

        if child_pidls.len() != paths.len() {
            return Err(Error::new(
                E_INVALIDARG,
                "Shell context menu selection was only partially resolved",
            ));
        }

        let Some(parent_folder) = parent_folder_opt else {
            return Err(Error::from_thread());
        };

        // Get IContextMenu
        let context_menu: IContextMenu = parent_folder.GetUIObjectOf(hwnd, &child_pidls, None)?;

        extract_context_menu(context_menu)
    }
}

/// Queries an already initialized Shell context-menu object and keeps all native
/// resources alive for lazy submenu loading and command invocation.
pub(crate) fn extract_context_menu(context_menu: IContextMenu) -> Result<ShellMenuContext> {
    unsafe {
        let hmenu = CreatePopupMenu()?;
        let query_started = std::time::Instant::now();
        if let Err(e) = context_menu
            .QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL)
            .ok()
        {
            let _ = DestroyMenu(hmenu);
            return Err(e);
        }
        let query_elapsed = query_started.elapsed();

        let count = GetMenuItemCount(Some(hmenu));
        log::debug!("[ShellMenu] Total menu items: {}", count);

        let materialize_started = std::time::Instant::now();
        let mut items = Vec::new();
        let mut pending_count = 0;
        for i in 0..count {
            if let Some(item) = extract_item_info(&context_menu, hmenu, i as u32, false) {
                if item.pending_submenu_handle.is_some() {
                    pending_count += 1;
                    log::trace!("[ShellMenu] Item '{}' has PENDING submenu", item.text);
                } else if !item.sub_items.is_empty() {
                    log::trace!(
                        "[ShellMenu] Item '{}' has {} sub-items",
                        item.text,
                        item.sub_items.len()
                    );
                }
                items.push(item);
            }
        }
        log::debug!(
            "[ShellMenu] Extracted {} items, {} with pending submenus",
            items.len(),
            pending_count
        );

        let materialize_elapsed = materialize_started.elapsed();
        log::debug!(
            "[ShellMenu] Query {:.1}ms, materialize {:.1}ms",
            query_elapsed.as_secs_f64() * 1000.0,
            materialize_elapsed.as_secs_f64() * 1000.0,
        );

        Ok(ShellMenuContext {
            items: std::cell::RefCell::new(items),
            context_menu,
            hmenu,
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

    let skip_icon =
        command_string.as_deref().is_some_and(is_known_verb) || is_filtered_shell_text(&text);
    let icon_rgba = if !skip_icon
        && !info.hbmpItem.0.is_null()
        && !std::ptr::eq(info.hbmpItem.0, HBMMENU_CALLBACK.0)
    {
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
        // QueryContextMenu() was called with idCmdFirst = 1.
        // InvokeCommand expects a zero-based offset encoded as MAKEINTRESOURCE.
        let command_offset = command_id.saturating_sub(1) as usize;

        // Use the real cursor position when available (screen coordinates),
        // because egui menu coordinates are not guaranteed to be absolute screen coords.
        let mut invoke_point = POINT {
            x: screen_x,
            y: screen_y,
        };
        let has_cursor_point = GetCursorPos(&mut invoke_point).is_ok();

        // Unicode + async improve compatibility with modern shell extensions (including cloud providers).
        let mut invoke_mask = SEE_MASK_UNICODE | SEE_MASK_ASYNCOK;
        if has_cursor_point {
            invoke_mask |= CMIC_MASK_PTINVOKE;
        }

        let invoke = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: invoke_mask,
            hwnd,
            lpVerb: PCSTR(command_offset as *const u8),
            lpVerbW: PCWSTR(command_offset as *const u16),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: invoke_point,
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

#[cfg(test)]
mod tests {
    use super::selection_has_single_parent;
    use std::path::PathBuf;

    #[test]
    fn shell_selection_requires_one_parent_folder() {
        assert!(selection_has_single_parent(&[
            PathBuf::from(r"C:\Folder\a.txt"),
            PathBuf::from(r"c:\folder\b.txt"),
        ]));
        assert!(!selection_has_single_parent(&[
            PathBuf::from(r"C:\Folder\a.txt"),
            PathBuf::from(r"C:\Other\b.txt"),
        ]));
    }
}
