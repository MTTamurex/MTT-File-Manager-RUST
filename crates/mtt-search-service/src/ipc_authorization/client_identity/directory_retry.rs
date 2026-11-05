//! Recover only an unavailable directory comparison, never an image rejection.
//! Anchor the service directory BEFORE impersonating; never re-resolve that
//! trusted reference through the client's path namespace.

use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

use super::{ClientIdentityError, PipeImpersonationGuard};

pub(super) fn verify_as_client(
    pipe: HANDLE,
    service_dir: &Path,
    client_dir: &Path,
) -> Result<(), ClientIdentityError> {
    let service = open_directory(service_dir)?;
    verify_with_anchor(pipe, &service, client_dir)
}

fn open_directory(path: &Path) -> Result<OwnedHandle, ClientIdentityError> {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path.as_ptr()),
            0x80, // FILE_READ_ATTRIBUTES only, not listing or file-content access.
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|error| inspection_error("Open directory", error))?;
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

fn verify_with_anchor(
    pipe: HANDLE,
    service: &OwnedHandle,
    client_dir: &Path,
) -> Result<(), ClientIdentityError> {
    let expected = directory_id(service)?;
    let _guard = PipeImpersonationGuard::new(pipe).map_err(ClientIdentityError::Unavailable)?;
    let client = open_directory(client_dir)?;
    let actual = directory_id(&client)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ClientIdentityError::Rejected(
            "Client directory identity differs from the anchored service directory".into(),
        ))
    }
}

fn directory_id(directory: &OwnedHandle) -> Result<(u64, [u8; 16]), ClientIdentityError> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(directory.as_raw_handle()), &mut info) }
        .map_err(|error| inspection_error("Query directory attributes", error))?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err(unavailable());
    }
    // Use the full file ID (also on ReFS), not a possibly truncated legacy ID.
    // Unsupported identity queries stay unavailable; never use lexical fallback.
    let mut id = FILE_ID_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(directory.as_raw_handle()),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    }
    .map_err(|error| inspection_error("Query full directory ID", error))?;
    if id.FileId.Identifier == [0; 16] {
        return Err(unavailable());
    }
    Ok((id.VolumeSerialNumber, id.FileId.Identifier))
}

fn unavailable() -> ClientIdentityError {
    ClientIdentityError::Unavailable("Cannot establish anchored directory identity".into())
}

fn inspection_error(operation: &str, error: windows::core::Error) -> ClientIdentityError {
    // Keep the failure stage/code useful for diagnosis without logging paths.
    ClientIdentityError::Unavailable(format!(
        "{operation} failed (HRESULT {:08x})",
        error.code().0,
    ))
}

#[cfg(test)]
mod tests;
