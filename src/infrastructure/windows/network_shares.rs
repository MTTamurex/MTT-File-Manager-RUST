//! SMB share enumeration. Call only from a background worker.

use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::domain::network_path::unc_server_name;
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_INVALID_DATA, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA};
use windows::Win32::NetworkManagement::NetManagement::{NetApiBufferFree, MAX_PREFERRED_LENGTH};
use windows::Win32::Storage::FileSystem::{
    NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE, STYPE_MASK, STYPE_SPECIAL,
};

struct ShareBuffer(*mut u8);

impl Drop for ShareBuffer {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = NetApiBufferFree(Some(self.0.cast()));
            }
        }
    }
}

fn share_entry(
    server: &str,
    name: String,
    share_type: u32,
    show_hidden: bool,
) -> Option<FileEntry> {
    let is_hidden = name.ends_with('$') || share_type & STYPE_SPECIAL.0 != 0;
    if share_type & STYPE_MASK.0 != STYPE_DISKTREE.0
        || (is_hidden && !show_hidden)
        || name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['\\', '/', '\0'])
    {
        return None;
    }
    // Do not query metadata: shares can require authentication or be unavailable.
    Some(FileEntry {
        path: PathBuf::from(format!(r"\\{server}\{name}")),
        name,
        is_dir: true,
        size: 0,
        modified: 0,
        created: None,
        folder_cover: None,
        drive_info: None,
        sync_status: SyncStatus::None,
        is_hidden,
        recycle_bin: None,
    })
}

pub fn list_network_shares(path: &str, show_hidden: bool) -> Result<Vec<FileEntry>, u32> {
    let server = unc_server_name(path).ok_or(ERROR_INVALID_PARAMETER.0)?;
    let server_wide: Vec<u16> = format!(r"\\{server}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut entries = Vec::new();
    let mut resume = 0;

    loop {
        let previous_resume = resume;
        let mut buffer = ShareBuffer(std::ptr::null_mut());
        let mut read = 0;
        let mut total = 0;
        let status = unsafe {
            NetShareEnum(
                PCWSTR(server_wide.as_ptr()),
                1,
                &mut buffer.0,
                MAX_PREFERRED_LENGTH,
                &mut read,
                &mut total,
                Some(&mut resume),
            )
        };
        if status != 0 && status != ERROR_MORE_DATA.0 {
            return Err(status);
        }
        if read > 0 {
            if buffer.0.is_null() {
                return Err(ERROR_INVALID_DATA.0);
            }
            // NetShareEnum level 1 supplies `read` SHARE_INFO_1 records owned by buffer.
            let shares = unsafe {
                std::slice::from_raw_parts(buffer.0.cast::<SHARE_INFO_1>(), read as usize)
            };
            for share in shares {
                if share.shi1_netname.is_null() {
                    return Err(ERROR_INVALID_DATA.0);
                }
                let name =
                    unsafe { share.shi1_netname.to_string() }.map_err(|_| ERROR_INVALID_DATA.0)?;
                if let Some(entry) = share_entry(server, name, share.shi1_type.0, show_hidden) {
                    entries.push(entry);
                }
            }
        }
        if status == 0 {
            return Ok(entries);
        }
        if read == 0 || resume == previous_resume {
            return Err(ERROR_INVALID_DATA.0);
        }
    }
}

#[cfg(test)]
#[path = "network_shares_tests.rs"]
mod tests;
