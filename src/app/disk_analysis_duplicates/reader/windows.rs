use super::{File, FileStamp, ReadOutcome};
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    self as fs, FileIoPriorityHintInfo, GetFileAttributesW, GetFileInformationByHandle,
    IoPriorityHintLow, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_IO_PRIORITY_HINT_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, PRIORITY_HINT,
};

const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;
const UNAVAILABLE: u32 = FILE_ATTRIBUTE_OFFLINE.0
    | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0
    | FILE_ATTRIBUTE_RECALL_ON_OPEN.0;

fn attributes(path: &Path) -> Result<u32, ReadOutcome> {
    let wide: Vec<_> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
    if attrs == fs::INVALID_FILE_ATTRIBUTES {
        Err(ReadOutcome::Inaccessible)
    } else {
        Ok(attrs)
    }
}

fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, ReadOutcome> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|_| ReadOutcome::Inaccessible)?;
    if info.dwFileAttributes & UNAVAILABLE != 0 {
        return Err(ReadOutcome::Unavailable);
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err(ReadOutcome::Changed);
    }
    Ok(info)
}

pub(super) fn capture(file: &File) -> Result<FileStamp, ReadOutcome> {
    let info = info(file)?;
    Ok(FileStamp {
        identity: [
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ],
        size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        creation: ((info.ftCreationTime.dwHighDateTime as u64) << 32)
            | info.ftCreationTime.dwLowDateTime as u64,
        modified: ((info.ftLastWriteTime.dwHighDateTime as u64) << 32)
            | info.ftLastWriteTime.dwLowDateTime as u64,
    })
}

pub(super) fn open(path: &Path) -> Result<(File, FileStamp), ReadOutcome> {
    // Fail closed if locality cannot be established. No read follows a
    // placeholder or a reparse point, including an ancestor directory.
    if attributes(path)? & UNAVAILABLE != 0 {
        return Err(ReadOutcome::Unavailable);
    }
    for ancestor in path.ancestors() {
        if attributes(ancestor)? & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(ReadOutcome::Changed);
        }
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
        .custom_flags(
            FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL,
        )
        .open(path)
        .map_err(|_| ReadOutcome::Inaccessible)?;
    let stamp = capture(&file)?;
    let hint = FILE_IO_PRIORITY_HINT_INFO {
        PriorityHint: PRIORITY_HINT(IoPriorityHintLow.0),
    };
    unsafe {
        let _ = SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileIoPriorityHintInfo,
            &hint as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&hint) as u32,
        );
    }
    Ok((file, stamp))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_attribute_masks_are_not_treated_as_local_data() {
        for attr in [
            FILE_ATTRIBUTE_OFFLINE.0,
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0,
            FILE_ATTRIBUTE_RECALL_ON_OPEN.0,
        ] {
            assert_ne!(attr & UNAVAILABLE, 0);
        }
    }
}
