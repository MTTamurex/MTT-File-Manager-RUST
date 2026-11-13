//! Validate the object authorized for replacement, not just its spelling.
use std::fs::{File, Metadata};
use std::io;
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    identity: (u64, u64),
    size: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
}

impl Snapshot {
    pub(crate) fn capture(path: &Path) -> io::Result<Self> {
        let file = open_target(path, false)?;
        Self::from_file(&file)
    }

    pub(crate) fn from_file(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() || crate::plan::is_link(&metadata) {
            return Err(io::Error::other("Replacement target is not a regular file"));
        }
        Ok(Self {
            identity: identity(file, &metadata)?,
            size: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
        })
    }

    pub(crate) fn hold(&self, path: &Path) -> io::Result<File> {
        crate::plan::check_path(path)?;
        let file = open_target(path, true)?;
        if Self::from_file(&file)? != *self {
            return Err(io::Error::other(
                "Replacement target changed after confirmation",
            ));
        }
        Ok(file)
    }

    pub(crate) fn validate(&self, path: &Path) -> io::Result<()> {
        self.hold(path).map(drop)
    }
}

fn open_target(path: &Path, lock_writes: bool) -> io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        };
        std::fs::OpenOptions::new()
            // Metadata-only handles do not enforce Windows read/write sharing.
            // Request data access for the guard so denying write sharing really
            // excludes existing and subsequent writers during verification.
            .access_mode(if lock_writes { FILE_READ_DATA.0 } else { 0 })
            .share_mode(
                FILE_SHARE_READ.0
                    | FILE_SHARE_DELETE.0
                    | if lock_writes { 0 } else { FILE_SHARE_WRITE.0 },
            )
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        let _ = lock_writes;
        File::open(path)
    }
}

pub(crate) fn identity(file: &File, metadata: &Metadata) -> io::Result<(u64, u64)> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let _ = metadata;
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xffff))?;
        Ok((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = file;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (file, metadata);
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub(crate) fn same_file(source: &Path, destination: &Path) -> io::Result<bool> {
    Ok(Snapshot::capture(source)?.identity == Snapshot::capture(destination)?.identity)
}

/// Pin destination ancestors against rename/reparse substitution during I/O.
pub(crate) fn pin_parents(path: &Path) -> io::Result<Vec<File>> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
            FILE_SHARE_WRITE,
        };
        let mut parents: Vec<_> = path.ancestors().collect();
        parents.reverse();
        let mut pins = Vec::with_capacity(parents.len());
        for parent in parents {
            let file = std::fs::OpenOptions::new()
                .access_mode(0)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(parent)?;
            let metadata = file.metadata()?;
            if !metadata.is_dir() || crate::plan::is_link(&metadata) {
                return Err(io::Error::other("Destination ancestor changed"));
            }
            pins.push(file);
        }
        Ok(pins)
    }
    #[cfg(not(windows))]
    {
        crate::plan::check_path(path)?;
        Ok(Vec::new())
    }
}
