use std::path::Path;
use windows::core::*;
use windows::Win32::Foundation::{E_ABORT, HWND};
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;

use super::ComApartmentGuard;
use super::RecycleBinShellItem;

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
            .ok_or_else(Error::from_thread)?;
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

/// Queue permanent removal of Recycle Bin namespace items through the Shell.
/// The caller owns the COM apartment and supplies a sink for themed progress.
pub(super) fn delete_recycle_bin_items_with_progress(
    hwnd: HWND,
    items: &[RecycleBinShellItem],
    sink: &IFileOperationProgressSink,
    item_sink: impl Fn(usize, &IShellItem) -> IFileOperationProgressSink,
    cancelled: impl Fn() -> bool,
) -> Result<bool> {
    unsafe {
        let file_op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        file_op.SetOwnerWindow(hwnd)?;
        file_op.SetOperationFlags(empty_bin_flags())?;
        // Item-specific sinks do not receive UpdateProgress/FinishOperations.
        let cookie = file_op.Advise(sink)?;
        let result = (|| {
            for (index, item) in items.iter().enumerate() {
                if cancelled() {
                    return Err(Error::from_hresult(E_ABORT));
                }
                file_op.DeleteItem(&item.item, &item_sink(index, &item.item))?;
            }
            if cancelled() {
                return Err(Error::from_hresult(E_ABORT));
            }
            file_op.PerformOperations()?;
            Ok(file_op.GetAnyOperationsAborted()?.as_bool())
        })();
        let unadvise = file_op.Unadvise(cookie);
        // Always disconnect, including setup failure/cancellation.
        result.and_then(|aborted| unadvise.map(|()| aborted))
    }
}

pub(super) fn empty_bin_flags() -> FILEOPERATION_FLAGS {
    FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_NO_CONNECTED_ELEMENTS | FOFX_EARLYFAILURE
}

/// Legacy synchronous API retained for callers that explicitly want Windows Shell UI.
pub(super) fn empty_recycle_bin(hwnd: HWND) -> Result<()> {
    unsafe {
        let _com = ComApartmentGuard::init_sta_best_effort();
        SHEmptyRecycleBinW(Some(hwnd), PCWSTR::default(), 0)?;
        Ok(())
    }
}

#[cfg(test)]
mod empty_bin_tests {
    use super::*;

    #[test]
    fn empty_recycle_bin_flags_suppress_native_ui_without_expanding_deletion_scope() {
        let flags = empty_bin_flags();
        assert_eq!(
            flags & (FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI),
            FOF_SILENT | FOF_NOCONFIRMATION | FOF_NOERRORUI
        );
        assert_ne!(flags & FOF_NO_CONNECTED_ELEMENTS, FILEOPERATION_FLAGS(0));
        assert_ne!(flags & FOFX_EARLYFAILURE, FILEOPERATION_FLAGS(0));
        assert_eq!(
            flags & (FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOF_WANTNUKEWARNING),
            FILEOPERATION_FLAGS(0)
        );
    }
}
