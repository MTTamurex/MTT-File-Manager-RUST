//! Native Windows `New` submenu support.
//!
//! The Shell owns the effective list, localization, ordering, and handlers for
//! this menu. Querying the folder background context menu avoids duplicating the
//! incomplete `ShellNew` registry rules in application code.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{IContextMenu, IShellFolder, SHBindToObject, SHParseDisplayName};

use crate::infrastructure::windows::native_menu::{extract_context_menu, ShellMenuContext};

struct PidlGuard(*mut ITEMIDLIST);

impl Drop for PidlGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
}

/// Extracts the native context menu for the empty area of `folder_path`.
/// Its `New` submenu remains backed by the original `IContextMenu`, allowing
/// Shell `Command` and `Handler` entries to be invoked exactly as in Explorer.
pub fn extract_background_menu(hwnd: HWND, folder_path: &Path) -> Result<ShellMenuContext> {
    let path_wide: Vec<u16> = folder_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mut pidl = std::ptr::null_mut();
        SHParseDisplayName(PCWSTR(path_wide.as_ptr()), None, &mut pidl, 0, None)?;
        let pidl = PidlGuard(pidl);
        let folder: IShellFolder = SHBindToObject(None, pidl.0, None)?;
        let context_menu: IContextMenu = folder.CreateViewObject(hwnd)?;
        extract_context_menu(context_menu)
    }
}
