use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, VOLUME_NAME_NONE,
    },
};

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(0)
        .share_mode(7)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
        .open(path)
}

pub(super) fn volume_relative_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut components = absolute.components();
    let prefix = match components.next() {
        Some(Component::Prefix(prefix))
            if matches!(
                prefix.kind(),
                Prefix::Disk(_)
                    | Prefix::VerbatimDisk(_)
                    | Prefix::UNC(_, _)
                    | Prefix::VerbatimUNC(_, _)
            ) =>
        {
            prefix
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unsupported copy path prefix",
            ))
        }
    };
    if components.next() != Some(Component::RootDir) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Copy path must have a volume root",
        ));
    }
    let file = open(&absolute)?;
    let mut buffer = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(HANDLE(file.as_raw_handle()), &mut buffer, VOLUME_NAME_NONE)
    } as usize;
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    if length >= buffer.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Resolved copy path is too long",
        ));
    }
    let relative = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
    let mut relative_components = relative.components();
    if relative_components.next() != Some(Component::RootDir) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Invalid volume-relative copy path",
        ));
    }
    let mut resolved = PathBuf::from(prefix.as_os_str());
    resolved.push(std::path::MAIN_SEPARATOR_STR);
    for component in relative_components {
        if !matches!(component, Component::Normal(_)) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Invalid volume-relative copy component",
            ));
        }
        resolved.push(component);
    }
    // A substituted drive or share rooted below the filesystem root may use a
    // different namespace base. Never accept a reconstructed path to another
    // object, even if it happens to exist and has the same filename/size.
    if identity_of(&file)? != identity(&resolved)? {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Volume-relative copy path does not identify the original file",
        ));
    }
    Ok(resolved)
}

#[derive(PartialEq, Eq)]
struct Identity {
    volume: u32,
    index: u64,
}

fn identity(path: &Path) -> io::Result<Identity> {
    let file = open(path)?;
    identity_of(&file)
}

fn identity_of(file: &File) -> io::Result<Identity> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xFFFF))?;
    let index = (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow);
    // WinFsp can legitimately report index zero for the volume root. Successful
    // queries still compare both the volume serial and index; do not reject it.
    Ok(Identity {
        volume: info.dwVolumeSerialNumber,
        index,
    })
}

pub(super) fn contains_directory(source: &Path, destination: &Path) -> io::Result<bool> {
    if !std::fs::metadata(source)?.is_dir() {
        return Ok(false);
    }
    let source_id = identity(source)?;
    for ancestor in destination.ancestors() {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if identity(ancestor)? == source_id {
            return Ok(true);
        }
    }
    Ok(false)
}
