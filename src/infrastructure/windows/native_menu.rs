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
    /// GDI bitmap handles extracted from menu items. Shell extensions create
    /// these during QueryContextMenu and many don't clean up on Release().
    /// We collect them here and delete AFTER DestroyMenu in Drop, because
    /// DeleteObject fails silently while the bitmap is still referenced by
    /// the live HMENU.
    /// RefCell for interior mutability (load_pending_submenu uses &self).
    owned_bitmaps: std::cell::RefCell<Vec<isize>>,
}

struct ComApartmentGuard {
    should_uninitialize: bool,
}

impl ComApartmentGuard {
    fn init_sta() -> Self {
        let should_uninitialize = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        Self {
            should_uninitialize,
        }
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }
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
            // 1. Destroy the menu first — releases Windows' internal references
            //    to the bitmaps set as hbmpItem.
            let _ = DestroyMenu(self.hmenu);

            // 2. NOW delete the orphaned GDI bitmaps. This must happen AFTER
            //    DestroyMenu; calling DeleteObject while the bitmap is still
            //    associated with a live HMENU fails silently (returns FALSE).
            for handle in self.owned_bitmaps.borrow().iter() {
                let hbmp = windows::Win32::Graphics::Gdi::HBITMAP(*handle as *mut _);
                let _ = windows::Win32::Graphics::Gdi::DeleteObject(hbmp.into());
            }
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
        log::debug!(
            "[ShellMenu] ===== EXTRACTION #{} for: {:?} items =====",
            call_num,
            paths.len()
        );

        // Parse all paths to PIDLs and collect children
        let mut pidls_to_free = PidlCleanupGuard::new(paths.len());
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
            return Err(Error::from_win32());
        }

        let Some(parent_folder) = parent_folder_opt else {
            return Err(Error::from_win32());
        };

        // Get IContextMenu
        let context_menu: IContextMenu = parent_folder.GetUIObjectOf(hwnd, &child_pidls, None)?;

        // Create popup menu to extract items
        let hmenu = CreatePopupMenu()?;
        if let Err(e) = context_menu
            .QueryContextMenu(hmenu, 0, 1, 0x7FFF, CMF_NORMAL)
            .ok()
        {
            let _ = DestroyMenu(hmenu);
            return Err(e);
        }

        let count = GetMenuItemCount(Some(hmenu));
        log::debug!("[ShellMenu] Total menu items: {}", count);

        // Extract items
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

        // Collect all GDI bitmap handles from menu items. They will be
        // deleted AFTER DestroyMenu in ShellMenuContext::Drop. Deleting
        // them while the HMENU is still alive fails silently.
        let owned_bitmaps = collect_menu_bitmaps(hmenu);
        log::debug!(
            "[ShellMenu] Collected {} GDI bitmap handles for deferred cleanup",
            owned_bitmaps.len()
        );

        Ok(ShellMenuContext {
            items: std::cell::RefCell::new(items),
            context_menu,
            hmenu,
            owned_bitmaps: std::cell::RefCell::new(owned_bitmaps),
        })
    }
}

/// Warmup function to pre-initialize shell extensions
/// Call this on app startup (e.g., with C:\ as path) to ensure
/// shell extensions like WinRAR, Send to, Include in library are loaded
pub fn warmup_shell_extensions(hwnd: HWND) {
    // Initialize COM on this thread (required for IShellFolder/IContextMenu).
    // Uses STA because shell extensions are apartment-threaded COM objects.
    let _com = ComApartmentGuard::init_sta();

    // Use the system drive root to trigger shell extension loading
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let warmup_path = std::path::PathBuf::from(format!("{}\\", system_drive));

    log::debug!(
        "[ShellMenu] Warming up shell extensions with {:?}...",
        warmup_path
    );

    if let Ok(_ctx) = extract_shell_menu(hwnd, &[warmup_path]) {
        log::debug!("[ShellMenu] Folder warmup complete");
    } else {
        log::warn!("[ShellMenu] Folder warmup failed");
    }

    // Use a temporary file to trigger file-level shell extensions (e.g., 7-Zip, WinRAR)
    let temp_file = std::env::temp_dir().join(format!("mtt_warmup_{}.txt", std::process::id()));
    let _ = std::fs::File::create(&temp_file);
    if let Ok(_ctx) = extract_shell_menu(hwnd, std::slice::from_ref(&temp_file)) {
        log::debug!("[ShellMenu] File warmup complete");
    } else {
        log::warn!("[ShellMenu] File warmup failed");
    }
    let _ = std::fs::remove_file(&temp_file);

    log::info!("[ShellMenu] Shell extensions initialized");
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

/// Recursively walk an HMENU and collect all real HBITMAP handles (as `isize`).
///
/// Windows' `DestroyMenu` does NOT free bitmaps set via `MENUITEMINFOW::hbmpItem`.
/// Shell extensions create these during `QueryContextMenu` and many do not clean
/// them up in their `IContextMenu::Release()` handler.
///
/// We collect them here so they can be deleted AFTER `DestroyMenu` in
/// `ShellMenuContext::Drop`. Deleting while the HMENU is alive fails silently.
///
/// Special system-defined bitmap constants (HBMMENU_*: values -1 through 11) are
/// skipped because they are not real GDI objects.
unsafe fn collect_menu_bitmaps(hmenu: HMENU) -> Vec<isize> {
    let mut handles = Vec::new();
    collect_menu_bitmaps_recursive(hmenu, &mut handles);
    handles
}

unsafe fn collect_menu_bitmaps_recursive(hmenu: HMENU, handles: &mut Vec<isize>) {
    let count = GetMenuItemCount(Some(hmenu));
    for i in 0..count {
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP | MIIM_SUBMENU,
            ..Default::default()
        };
        if GetMenuItemInfoW(hmenu, i as u32, true, &mut info).is_ok() {
            let ptr = info.hbmpItem.0 as isize;
            if !info.hbmpItem.0.is_null() && !((-1..=11).contains(&ptr)) {
                handles.push(ptr);
            }
            if !info.hSubMenu.0.is_null() {
                collect_menu_bitmaps_recursive(info.hSubMenu, handles);
            }
        }
    }
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

                // Collect new bitmaps from the lazy-loaded submenu for
                // deferred deletion in Drop (same rationale as initial extraction).
                let new_bitmaps = collect_menu_bitmaps(hsubmenu);
                self.owned_bitmaps.borrow_mut().extend(new_bitmaps);

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
