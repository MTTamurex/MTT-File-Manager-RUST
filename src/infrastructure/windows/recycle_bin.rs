//! Windows Recycle Bin implementation.
//! Uses IShellItem2 to retrieve robust metadata (original path, deletion date).

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::*;

mod enumeration;
mod operations;

// Property keys for Recycle Bin items.
pub const PKEY_SIZE: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
    pid: 12,
};
// System.Recycle.DeletedFrom - the original folder location.
pub const PKEY_RECYCLE_DELETED_FROM: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B174B33_40FF_11D2_A27E_00C04FC30871),
    pid: 2,
};
// System.Recycle.DateDeleted.
pub const PKEY_RECYCLE_DATE_DELETED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9B174B33_40FF_11D2_A27E_00C04FC30871),
    pid: 3,
};
// System.ItemNameDisplay - the display name.
pub const PKEY_ITEMNAMEDISPLAY: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC),
    pid: 10,
};

/// Represents an item in the Recycle Bin.
#[derive(Debug, Clone)]
pub struct RecycleBinItem {
    /// Display name (filename only, e.g., "document.docx").
    pub name: String,
    /// Parent folder where the item was deleted from (e.g., "C:\Users\Documents").
    pub parent_folder: String,
    /// Full original path for restoration (e.g., "C:\Users\Documents\document.docx").
    pub original_path: PathBuf,
    /// Physical path in $Recycle.Bin (e.g., "C:\$Recycle.Bin\...\$R123456.docx").
    pub physical_path: PathBuf,
    /// Date when item was deleted.
    pub date_deleted: String,
    /// Deletion timestamp in UNIX seconds (0 when unavailable).
    pub date_deleted_unix: u64,
    /// Size in bytes.
    pub size: u64,
    /// Whether item is a directory.
    pub is_directory: bool,
    /// File extension for icon lookup (e.g., ".docx").
    pub extension: String,
}

/// RAII guard for COM apartment initialization on the current thread.
pub(crate) struct ComApartmentGuard {
    initialized: bool,
}

impl ComApartmentGuard {
    pub(crate) fn init_sta_best_effort() -> Self {
        let initialized = unsafe {
            // SAFETY: Initializes COM apartment for the current thread.
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok()
        };
        Self { initialized }
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                // SAFETY: Balanced with successful CoInitializeEx in init_sta_best_effort.
                CoUninitialize();
            }
        }
    }
}

/// Retrieves the total count and size of items in the Recycle Bin.
pub fn get_recycle_bin_info() -> Result<(u64, u64)> {
    unsafe {
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..Default::default()
        };

        SHQueryRecycleBinW(PCWSTR::default(), &mut info)?;
        Ok((info.i64NumItems as u64, info.i64Size as u64))
    }
}

pub fn enumerate_recycle_bin_streaming(
    sender: Sender<Vec<RecycleBinItem>>,
    generation: Arc<AtomicUsize>,
    my_gen: usize,
    batch_size: usize,
) {
    enumeration::enumerate_recycle_bin_streaming(sender, generation, my_gen, batch_size);
}

/// Legacy function for backwards compatibility - enumerates all at once.
pub fn enumerate_recycle_bin() -> Result<Vec<RecycleBinItem>> {
    enumeration::enumerate_recycle_bin()
}

/// Restore a file from the Recycle Bin to its original location.
pub fn restore_from_recycle_bin(
    physical_path: &std::path::Path,
    original_path: &std::path::Path,
) -> Result<()> {
    operations::restore_from_recycle_bin(physical_path, original_path)
}

/// Permanently delete a file from the Recycle Bin.
/// Shows native Windows confirmation dialog before deleting.
pub fn delete_permanently(physical_path: &std::path::Path, hwnd: HWND) -> Result<()> {
    operations::delete_permanently(physical_path, hwnd)
}

/// Empty the entire Recycle Bin.
/// Shows native Windows confirmation dialog before emptying.
pub fn empty_recycle_bin(hwnd: HWND) -> Result<()> {
    operations::empty_recycle_bin(hwnd)
}
