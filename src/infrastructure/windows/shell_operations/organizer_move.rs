use std::path::Path;
use windows::Win32::Storage::FileSystem::{
    BackupRead, BackupWrite, FileDispositionInfo, FileRenameInfo, GetFileInformationByHandle,
    GetFinalPathNameByHandleW, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, DELETE,
    FILE_DISPOSITION_INFO, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_NAME_NORMALIZED,
    FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrganizerFileSnapshot {
    size: u64,
    creation_time: u64,
    last_write_time: u64,
    volume_serial_number: u32,
    file_index: u64,
}

impl OrganizerFileSnapshot {
    pub(crate) fn to_bytes(self) -> [u8; 36] {
        let mut bytes = [0u8; 36];
        bytes[0..8].copy_from_slice(&self.size.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.creation_time.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.last_write_time.to_le_bytes());
        bytes[24..28].copy_from_slice(&self.volume_serial_number.to_le_bytes());
        bytes[28..36].copy_from_slice(&self.file_index.to_le_bytes());
        bytes
    }

    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != 36 {
            return None;
        }
        Some(Self {
            size: u64::from_le_bytes(bytes[0..8].try_into().ok()?),
            creation_time: u64::from_le_bytes(bytes[8..16].try_into().ok()?),
            last_write_time: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
            volume_serial_number: u32::from_le_bytes(bytes[24..28].try_into().ok()?),
            file_index: u64::from_le_bytes(bytes[28..36].try_into().ok()?),
        })
    }
}

fn snapshot_from_file(file: &std::fs::File) -> std::io::Result<OrganizerFileSnapshot> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
            .map_err(|_| std::io::Error::last_os_error())?;
    }

    Ok(OrganizerFileSnapshot {
        size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        creation_time: ((info.ftCreationTime.dwHighDateTime as u64) << 32)
            | info.ftCreationTime.dwLowDateTime as u64,
        last_write_time: ((info.ftLastWriteTime.dwHighDateTime as u64) << 32)
            | info.ftLastWriteTime.dwLowDateTime as u64,
        volume_serial_number: info.dwVolumeSerialNumber,
        file_index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}

pub fn organizer_file_snapshot(path: &Path) -> std::io::Result<OrganizerFileSnapshot> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .open(path)?;
    snapshot_from_file(&file)
}

pub(crate) fn remove_organizer_file_if_matches(
    path: &Path,
    expected: OrganizerFileSnapshot,
) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)?;
    if snapshot_from_file(&file)? != expected {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "organizer file changed before cleanup",
        ));
    }
    mark_file_for_deletion(&file)
}

/// Moves a file without replacing an existing destination.
pub fn move_file_without_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    move_file_without_replace_impl(source, destination, None, |_| Ok(())).map(|_| ())
}

pub fn move_organizer_file_without_replace(
    source: &Path,
    destination: &Path,
    expected: OrganizerFileSnapshot,
) -> std::io::Result<OrganizerFileSnapshot> {
    move_file_without_replace_impl(source, destination, Some(expected), |_| Ok(()))
}

pub(crate) fn move_organizer_file_without_replace_journaled(
    source: &Path,
    destination: &Path,
    expected: OrganizerFileSnapshot,
    journal: impl FnMut(OrganizerFileSnapshot) -> std::io::Result<()>,
) -> std::io::Result<OrganizerFileSnapshot> {
    move_file_without_replace_impl(source, destination, Some(expected), journal)
}

pub fn move_organizer_file_without_replace_guarded_by(
    source: &Path,
    destination: &Path,
    expected_source: OrganizerFileSnapshot,
    guard_path: &Path,
    expected_guard: OrganizerFileSnapshot,
) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    let guard = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ.0)
        .open(guard_path)?;
    if snapshot_from_file(&guard)? != expected_guard {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "guarded organizer file changed before the move",
        ));
    }
    move_file_without_replace_impl(source, destination, Some(expected_source), |_| Ok(()))
        .map(|_| ())
}

fn move_file_without_replace_impl<F>(
    source: &Path,
    destination: &Path,
    expected: Option<OrganizerFileSnapshot>,
    mut before_publish: F,
) -> std::io::Result<OrganizerFileSnapshot>
where
    F: FnMut(OrganizerFileSnapshot) -> std::io::Result<()>,
{
    use std::os::windows::fs::OpenOptionsExt;

    // DELETE access plus no write/delete sharing locks this exact file identity
    // against modification, rename, deletion, or pathname replacement.
    let mut source_guard = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ.0 | DELETE.0)
        .share_mode(FILE_SHARE_READ.0)
        .open(source)?;
    let source_snapshot = snapshot_from_file(&source_guard)?;
    if let Some(expected) = expected {
        if source_snapshot != expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "source file changed before the organizer move",
            ));
        }
    }
    let destination_parent_guard = open_destination_parent(destination)?;
    before_publish(source_snapshot)?;

    // Let the filesystem determine whether this is a native rename. Only an
    // explicit cross-device error enters the verified copy/delete fallback.
    match rename_file_handle(&source_guard, destination) {
        Ok(()) => return Ok(source_snapshot),
        Err(error) if error.raw_os_error() == Some(17) => {}
        Err(error) => return Err(error),
    }

    move_across_volumes_verified(
        &mut source_guard,
        destination,
        &destination_parent_guard,
        &mut before_publish,
    )
}

fn open_destination_parent(destination: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    if !destination.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "organizer destination must be absolute",
        ));
    }
    let parent_path = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "organizer destination has no parent",
        )
    })?;
    destination.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "organizer destination has no file name",
        )
    })?;
    let parent = std::fs::OpenOptions::new()
        .access_mode(FILE_GENERIC_READ.0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(parent_path)?;
    if !same_windows_path(&final_path_from_file(&parent)?, parent_path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "organizer destination directory changed during validation",
        ));
    }
    Ok(parent)
}

fn final_path_from_file(file: &std::fs::File) -> std::io::Result<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;

    let mut buffer = vec![0u16; 512];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                HANDLE(file.as_raw_handle()),
                &mut buffer,
                FILE_NAME_NORMALIZED,
            )
        };
        if length == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if (length as usize) < buffer.len() {
            return Ok(std::path::PathBuf::from(std::ffi::OsString::from_wide(
                &buffer[..length as usize],
            )));
        }
        buffer.resize(length as usize + 1, 0);
    }
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    fn comparable(path: &Path) -> String {
        let path = path.to_string_lossy();
        let path = path
            .strip_prefix(r"\\?\UNC\")
            .map(|path| format!(r"\\{path}"))
            .or_else(|| path.strip_prefix(r"\\?\").map(str::to_owned))
            .unwrap_or_else(|| path.into_owned());
        path.trim_end_matches(['\\', '/']).to_owned()
    }

    comparable(left).eq_ignore_ascii_case(&comparable(right))
}

fn move_across_volumes_verified<F>(
    source: &mut std::fs::File,
    destination: &Path,
    _destination_parent_guard: &std::fs::File,
    before_publish: &mut F,
) -> std::io::Result<OrganizerFileSnapshot>
where
    F: FnMut(OrganizerFileSnapshot) -> std::io::Result<()>,
{
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let destination_parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination has no parent",
        )
    })?;
    destination.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "destination has no file name",
        )
    })?;
    let mut temporary = None;
    for _ in 0..32 {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let candidate =
            destination_parent.join(format!(".mtt-organizer-{}-{id}.tmp", std::process::id()));
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | DELETE.0)
            .share_mode(FILE_SHARE_READ.0)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let (_temp_path, mut copied) = temporary.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not reserve a temporary organizer destination",
        )
    })?;

    let publish_result = (|| {
        backup_copy_all_streams(source, &copied)?;
        let source_size = source.metadata()?.len();
        let source_hash = hash_open_file(source)?;
        copied.sync_all()?;
        if copied.metadata()?.len() != source_size || hash_open_file(&mut copied)? != source_hash {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "cross-volume organizer copy failed integrity validation",
            ));
        }
        let destination_snapshot = snapshot_from_file(&copied)?;
        before_publish(destination_snapshot)?;
        rename_file_handle(&copied, destination)?;
        copied.sync_all()?;
        Ok(destination_snapshot)
    })();
    let destination_snapshot = match publish_result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = mark_file_for_deletion(&copied);
            return Err(error);
        }
    };

    // The final destination data and rename have both been flushed. Delete the exact
    // guarded source identity rather than whatever currently occupies its path.
    if let Err(error) = mark_file_for_deletion(source) {
        if let Err(cleanup_error) = mark_file_for_deletion(&copied) {
            return Err(std::io::Error::new(
                error.kind(),
                format!(
                    "source deletion failed after cross-volume publish: {error}; \
                     destination cleanup also failed: {cleanup_error}"
                ),
            ));
        }
        return Err(error);
    }
    Ok(destination_snapshot)
}

fn rename_file_handle(file: &std::fs::File, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;

    if !destination.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "organizer destination must be absolute",
        ));
    }
    let destination_wide: Vec<u16> = destination.as_os_str().encode_wide().collect();
    let header_size = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let file_name_bytes = destination_wide.len() * std::mem::size_of::<u16>();
    // FILE_RENAME_INFO uses FileNameLength, but some filesystems still inspect
    // the trailing WCHAR. Keep an explicit NUL inside the allocated buffer.
    let buffer_size = header_size + file_name_bytes + std::mem::size_of::<u16>();
    let words = buffer_size.div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0usize; words];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();

    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = HANDLE::default();
        (*info).FileNameLength = file_name_bytes as u32;
        std::ptr::copy_nonoverlapping(
            destination_wide.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            destination_wide.len(),
        );
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileRenameInfo,
            info.cast(),
            buffer_size as u32,
        )
        .map_err(|_| std::io::Error::last_os_error())
    }
}

fn mark_file_for_deletion(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;

    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    unsafe {
        SetFileInformationByHandle(
            HANDLE(file.as_raw_handle()),
            FileDispositionInfo,
            std::ptr::addr_of!(disposition).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
        .map_err(|_| std::io::Error::last_os_error())
    }
}

fn hash_open_file(file: &mut std::fs::File) -> std::io::Result<blake3::Hash> {
    use std::io::{Read, Seek, SeekFrom};

    file.seek(SeekFrom::Start(0))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

fn backup_copy_all_streams(
    source: &std::fs::File,
    destination: &std::fs::File,
) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;

    let source_handle = HANDLE(source.as_raw_handle());
    let destination_handle = HANDLE(destination.as_raw_handle());
    let mut read_context = std::ptr::null_mut();
    let mut write_context = std::ptr::null_mut();
    let mut buffer = vec![0u8; 1024 * 1024];

    let result = (|| {
        loop {
            let mut bytes_read = 0u32;
            unsafe {
                BackupRead(
                    source_handle,
                    &mut buffer,
                    &mut bytes_read,
                    false,
                    false,
                    &mut read_context,
                )
                .map_err(|_| std::io::Error::last_os_error())?;
            }
            if bytes_read == 0 {
                break;
            }

            let mut offset = 0usize;
            while offset < bytes_read as usize {
                let mut bytes_written = 0u32;
                unsafe {
                    BackupWrite(
                        destination_handle,
                        &buffer[offset..bytes_read as usize],
                        &mut bytes_written,
                        false,
                        false,
                        &mut write_context,
                    )
                    .map_err(|_| std::io::Error::last_os_error())?;
                }
                if bytes_written == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "BackupWrite made no progress",
                    ));
                }
                offset += bytes_written as usize;
            }
        }
        Ok(())
    })();

    let mut ignored = 0u32;
    unsafe {
        let _ = BackupRead(
            source_handle,
            &mut [],
            &mut ignored,
            true,
            false,
            &mut read_context,
        );
        let _ = BackupWrite(
            destination_handle,
            &[],
            &mut ignored,
            true,
            false,
            &mut write_context,
        );
    }
    result
}

#[cfg(test)]
mod tests;
