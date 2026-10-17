use super::{extract_context_menu, PidlCleanupGuard, ShellMenuContext};
use std::path::PathBuf;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;

/// Resolve roots relative to This PC instead of parsing absolute paths. In
/// particular, mapped drives can otherwise resolve into the Network namespace.
pub(super) fn extract(hwnd: HWND, paths: &[PathBuf]) -> Result<ShellMenuContext> {
    unsafe {
        let mut pidls = PidlCleanupGuard::new(paths.len() + 1);
        let computer_pidl = SHGetKnownFolderIDList(&FOLDERID_ComputerFolder, 0, None)?;
        pidls.push(computer_pidl);
        let computer: IShellFolder = SHBindToObject(None, computer_pidl, None)?;
        let mut children = Vec::with_capacity(paths.len());

        for path in paths {
            let root = crate::infrastructure::windows::normalize_drive_root_path(path)
                .ok_or_else(|| Error::new(E_INVALIDARG, "Expected a drive root"))?;
            let mut name: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
            let parsed = computer.ParseDisplayName(
                hwnd,
                None,
                PWSTR(name.as_mut_ptr()),
                None,
                &mut child,
                std::ptr::null_mut(),
            );
            if !child.is_null() {
                pidls.push(child);
            }
            parsed?;
            if child.is_null() {
                return Err(Error::new(E_INVALIDARG, "Shell returned a null drive item"));
            }
            children.push(child as *const ITEMIDLIST);
        }

        let context_menu: IContextMenu = computer.GetUIObjectOf(hwnd, &children, None)?;
        extract_context_menu(context_menu)
    }
}
