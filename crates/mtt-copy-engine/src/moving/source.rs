//! Windows handle-based rename/disposition. Never remove a path by its name.
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct Directory {
    pub path: PathBuf,
    pub destination: PathBuf,
    identity: (u64, u64),
}

impl Directory {
    pub(crate) fn capture(path: &Path, destination: &Path) -> io::Result<Self> {
        crate::plan::check_path(path)?;
        let file = metadata_directory(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir() || crate::plan::is_link(&metadata) {
            return Err(io::Error::other("Move directory changed"));
        }
        Ok(Self {
            path: path.to_owned(),
            destination: destination.to_owned(),
            identity: crate::replacement::identity(&file, &metadata)?,
        })
    }

    pub(crate) fn remove_empty(&self) -> io::Result<()> {
        let _pins = pin_parents(
            self.path
                .parent()
                .ok_or_else(|| io::Error::other("Invalid source parent"))?,
        )?;
        let file = open(&self.path, true)?;
        let metadata = file.metadata()?;
        if !metadata.is_dir()
            || crate::plan::is_link(&metadata)
            || crate::replacement::identity(&file, &metadata)? != self.identity
        {
            return Err(io::Error::other("Move directory changed; retained"));
        }
        remove(&file)
    }
}

#[cfg(windows)]
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
#[cfg(windows)]
use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};

pub(crate) fn open(path: &Path, directory: bool) -> io::Result<File> {
    crate::plan::check_path(path)?;
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new()
            .access_mode(FILE_READ_DATA.0 | DELETE.0)
            .share_mode(if directory {
                FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0
            } else {
                FILE_SHARE_READ.0
            })
            .custom_flags(
                FILE_FLAG_OPEN_REPARSE_POINT.0
                    | if directory {
                        FILE_FLAG_BACKUP_SEMANTICS.0
                    } else {
                        0
                    },
            )
            .open(path)
    }
    #[cfg(not(windows))]
    {
        let _ = directory;
        Err(io::ErrorKind::Unsupported.into())
    }
}

fn metadata_directory(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        File::open(path)
    }
}

pub(crate) fn pin_parents(path: &Path) -> io::Result<Vec<File>> {
    #[cfg(windows)]
    {
        let mut ancestors: Vec<_> = path.ancestors().collect();
        ancestors.reverse();
        let mut pins = Vec::new();
        for ancestor in ancestors {
            let file = std::fs::OpenOptions::new()
                .access_mode(FILE_LIST_DIRECTORY.0)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0 | FILE_FLAG_BACKUP_SEMANTICS.0)
                .open(ancestor)?;
            let metadata = file.metadata()?;
            if !metadata.is_dir() || crate::plan::is_link(&metadata) {
                return Err(io::Error::other("Move ancestor changed"));
            }
            pins.push(file);
        }
        Ok(pins)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub(crate) fn protect_destination(path: &Path) -> io::Result<File> {
    #[cfg(windows)]
    {
        crate::plan::check_path(path)?;
        std::fs::OpenOptions::new()
            .access_mode(FILE_READ_DATA.0)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub(crate) fn remove(file: &File) -> io::Result<()> {
    #[cfg(windows)]
    {
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        }
        .map_err(os_error)
    }
    #[cfg(not(windows))]
    {
        let _ = file;
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub(crate) fn rename(file: &File, destination: &Path, replace: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        // Win32's DOS-path conversion also requires a terminating NUL, even
        // though FileNameLength excludes it. Do not rely on allocation padding.
        let name: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let size = std::mem::size_of::<FILE_RENAME_INFO>().max(offset + name.len() * 2);
        let size_u32 = u32::try_from(size).map_err(|_| io::Error::other("Move path too long"))?;
        // usize storage supplies alignment for FILE_RENAME_INFO and its trailing
        // UTF-16 name. The initialized allocation remains alive through Win32.
        let mut storage = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*info).Anonymous.ReplaceIfExists = replace;
            (*info).FileNameLength = ((name.len() - 1) * 2) as u32;
            std::ptr::copy_nonoverlapping(
                name.as_ptr(),
                storage.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                name.len(),
            );
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileRenameInfo,
                info.cast(),
                size_u32,
            )
        }
        .map_err(os_error)
    }
    #[cfg(not(windows))]
    {
        let _ = (file, destination, replace);
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(windows)]
fn os_error(error: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(error.code().0 & 0xffff)
}
