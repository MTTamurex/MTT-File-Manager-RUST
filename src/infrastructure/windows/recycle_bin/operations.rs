use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::{E_ABORT, HWND};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;

use super::ComApartmentGuard;

/// Restore a file from the Recycle Bin to its original location.
pub(super) fn restore_from_recycle_bin(physical_path: &Path, original_path: &Path) -> Result<()> {
    unsafe {
        let _com = ComApartmentGuard::init_sta_best_effort();

        // Use IFileOperation for undo/restore.
        let file_op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;

        // Set operation flags.
        file_op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT)?;

        // Create shell item for the physical path ($R file).
        let physical_str = physical_path.to_string_lossy();
        let physical_wide: Vec<u16> = physical_str.encode_utf16().chain(Some(0)).collect();
        let source_item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR::from_raw(physical_wide.as_ptr()), None)?;

        // Create shell item for destination folder.
        let dest_folder = original_path.parent().unwrap_or(original_path);
        let dest_str = dest_folder.to_string_lossy();
        let dest_wide: Vec<u16> = dest_str.encode_utf16().chain(Some(0)).collect();
        let dest_folder_item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR::from_raw(dest_wide.as_ptr()), None)?;

        // Get original filename.
        let file_name = original_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(Error::from_win32)?;
        let name_wide: Vec<u16> = file_name.encode_utf16().chain(Some(0)).collect();

        // Move the item.
        file_op.MoveItem(
            &source_item,
            &dest_folder_item,
            PCWSTR::from_raw(name_wide.as_ptr()),
            None,
        )?;
        file_op.PerformOperations()?;

        Ok(())
    }
}

/// Permanently delete a file from the Recycle Bin.
/// Shows native Windows confirmation dialog before deleting.
pub(super) fn delete_permanently(physical_path: &Path, hwnd: HWND) -> Result<()> {
    unsafe {
        let _com = ComApartmentGuard::init_sta_best_effort();

        // Use IFileOperation for permanent deletion.
        let file_op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;

        // Set owner window for the confirmation dialog.
        file_op.SetOwnerWindow(hwnd)?;

        // No FOF_ALLOWUNDO = permanent. No FOF_NOCONFIRMATION = Windows shows confirmation.
        file_op.SetOperationFlags(FOF_NOERRORUI)?;

        // Create shell item for the physical path.
        let path_str = physical_path.to_string_lossy();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(Some(0)).collect();
        let item: IShellItem =
            SHCreateItemFromParsingName(PCWSTR::from_raw(path_wide.as_ptr()), None)?;

        // Delete the item permanently.
        file_op.DeleteItem(&item, None)?;
        file_op.PerformOperations()?;

        // Check if user cancelled.
        if file_op.GetAnyOperationsAborted()?.as_bool() {
            return Err(Error::from_hresult(E_ABORT));
        }

        Ok(())
    }
}

/// Empty the entire Recycle Bin.
/// Shows native Windows confirmation dialog before emptying.
pub(super) fn empty_recycle_bin(hwnd: HWND) -> Result<()> {
    unsafe {
        // SHEmptyRecycleBinW with NULL path empties all drives.
        // No SHERB_NOCONFIRMATION = Windows shows native confirmation dialog.
        SHEmptyRecycleBinW(Some(hwnd), PCWSTR::default(), SHERB_NOPROGRESSUI)?;
        Ok(())
    }
}
