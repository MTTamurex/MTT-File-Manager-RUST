use crate::image_viewer::loader::{self, DecodedFrame, ExportImageFormat};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSnapshot {
    size: u64,
    creation_time: u64,
    last_write_time: u64,
    change_time: i64,
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(target_os = "windows")]
impl FileSnapshot {
    fn same_file_identity(self, other: Self) -> bool {
        self.volume_serial_number == other.volume_serial_number
            && self.file_index == other.file_index
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSnapshot {
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    device: u64,
    inode: u64,
}

#[cfg(all(not(target_os = "windows"), not(unix)))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileSnapshot {
    size: u64,
    modified: Option<std::time::SystemTime>,
}

#[cfg(target_os = "windows")]
pub fn capture_file_snapshot(path: &Path) -> io::Result<FileSnapshot> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .open(path)?;
    snapshot_from_file(&file)
}

#[cfg(target_os = "windows")]
fn snapshot_from_file(file: &std::fs::File) -> io::Result<FileSnapshot> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO,
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
            .map_err(|_| io::Error::last_os_error())?;
    }
    let mut basic = FILE_BASIC_INFO::default();
    unsafe {
        GetFileInformationByHandleEx(
            HANDLE(file.as_raw_handle()),
            FileBasicInfo,
            std::ptr::addr_of_mut!(basic).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
        .map_err(|_| io::Error::last_os_error())?;
    }

    Ok(FileSnapshot {
        size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
        creation_time: ((info.ftCreationTime.dwHighDateTime as u64) << 32)
            | info.ftCreationTime.dwLowDateTime as u64,
        last_write_time: ((info.ftLastWriteTime.dwHighDateTime as u64) << 32)
            | info.ftLastWriteTime.dwLowDateTime as u64,
        change_time: basic.ChangeTime,
        volume_serial_number: info.dwVolumeSerialNumber,
        file_index: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}

pub struct SourceGuard {
    _file: std::fs::File,
}

#[cfg(target_os = "windows")]
pub fn guard_unchanged_source(path: &Path, expected: FileSnapshot) -> io::Result<SourceGuard> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;

    // Sharing reads only pins this file identity and prevents writes, renames,
    // and deletion while decoders reopen the same path.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(path)?;
    if snapshot_from_file(&file)? != expected {
        return Err(file_changed_error());
    }
    Ok(SourceGuard { _file: file })
}

#[cfg(not(target_os = "windows"))]
impl SourceGuard {
    pub fn read_all(&self) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};

        let mut file = self._file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(not(target_os = "windows"))]
pub fn guard_unchanged_source(path: &Path, expected: FileSnapshot) -> io::Result<SourceGuard> {
    let file = std::fs::File::open(path)?;
    ensure_file_unchanged(path, expected)?;
    Ok(SourceGuard { _file: file })
}

#[cfg(unix)]
pub fn capture_file_snapshot(path: &Path) -> io::Result<FileSnapshot> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path)?;
    Ok(FileSnapshot {
        size: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(all(not(target_os = "windows"), not(unix)))]
pub fn capture_file_snapshot(path: &Path) -> io::Result<FileSnapshot> {
    let metadata = std::fs::metadata(path)?;
    Ok(FileSnapshot {
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

pub fn ensure_file_unchanged(path: &Path, expected: FileSnapshot) -> io::Result<()> {
    if capture_file_snapshot(path)? == expected {
        Ok(())
    } else {
        Err(file_changed_error())
    }
}

fn file_changed_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "File changed after the crop was confirmed",
    )
}

pub fn save_frame_atomically(
    frame: DecodedFrame,
    format: ExportImageFormat,
    destination: &Path,
    replace_existing: bool,
    expected_destination: Option<FileSnapshot>,
) -> io::Result<()> {
    verify_destination(destination, replace_existing, expected_destination)?;
    let (temporary, file) = reserve_temporary_path(destination, replace_existing)?;
    let result = (|| {
        loader::encode_frame_to_file(frame, format, file)?;
        preserve_destination_permissions(destination, &temporary, replace_existing)?;
        verify_destination(destination, replace_existing, expected_destination)?;
        publish_temporary(
            &temporary,
            destination,
            replace_existing,
            expected_destination,
        )
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(unix)]
fn preserve_destination_permissions(
    destination: &Path,
    temporary: &Path,
    replace_existing: bool,
) -> io::Result<()> {
    if replace_existing {
        std::fs::set_permissions(temporary, std::fs::metadata(destination)?.permissions())?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn preserve_destination_permissions(
    _destination: &Path,
    _temporary: &Path,
    _replace_existing: bool,
) -> io::Result<()> {
    Ok(())
}

fn verify_destination(
    destination: &Path,
    replace_existing: bool,
    expected: Option<FileSnapshot>,
) -> io::Result<()> {
    match (replace_existing, expected) {
        (true, Some(expected)) => ensure_file_unchanged(destination, expected),
        (true, None) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Replacing a file requires a confirmed file snapshot",
        )),
        (false, _) if destination.exists() => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Destination was created before the cropped image could be saved",
        )),
        (false, _) => Ok(()),
    }
}

fn reserve_temporary_path(
    destination: &Path,
    restrict_permissions: bool,
) -> io::Result<(PathBuf, std::fs::File)> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Destination has no parent directory",
        )
    })?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    cleanup_stale_crop_files(destination, parent, name);

    for _ in 0..100 {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{name}.mtt-crop-{}-{sequence}.tmp",
            std::process::id()
        ));
        match create_temporary(&candidate, restrict_permissions) {
            Ok(file) => {
                return Ok((candidate, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Could not reserve a temporary output file",
    ))
}

fn cleanup_stale_crop_files(destination: &Path, parent: &Path, destination_name: &str) {
    const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    let prefix = format!(".{destination_name}.mtt-crop-");
    let backup_prefix = format!(".{destination_name}.mtt-crop-backup-");
    let conflict_prefix = format!(".{destination_name}.mtt-crop-conflict-");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) || !name.ends_with(".tmp") {
            continue;
        }
        if name.starts_with(&backup_prefix) {
            if !destination.exists() {
                match std::fs::rename(entry.path(), destination) {
                    Ok(()) => log::warn!(
                        "[IMAGE-VIEWER] restored interrupted crop backup for '{}'",
                        destination.display()
                    ),
                    Err(error) => log::warn!(
                        "[IMAGE-VIEWER] failed to restore interrupted crop backup '{}': {}",
                        entry.path().display(),
                        error
                    ),
                }
            }
            continue;
        }
        if name.starts_with(&conflict_prefix) {
            continue;
        }
        let is_stale = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_AFTER);
        if is_stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn create_temporary(path: &Path, restrict_permissions: bool) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(if restrict_permissions { 0o600 } else { 0o666 })
        .open(path)
}

#[cfg(target_os = "windows")]
fn create_temporary(path: &Path, restrict_permissions: bool) -> io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    if restrict_permissions && !crate::infrastructure::db_utils::harden_file_permissions(path) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Could not restrict temporary image permissions",
        ));
    }
    Ok(file)
}

#[cfg(all(not(unix), not(target_os = "windows")))]
fn create_temporary(path: &Path, _restrict_permissions: bool) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(target_os = "windows")]
fn publish_temporary(
    temporary: &Path,
    destination: &Path,
    replace_existing: bool,
    expected_destination: Option<FileSnapshot>,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH};

    let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if !replace_existing {
        return unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|error| io::Error::other(error.to_string()));
    }

    let expected = expected_destination.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Replacing a file requires a confirmed file snapshot",
        )
    })?;
    use std::os::windows::fs::OpenOptionsExt;
    use windows::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    let destination_guard = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_DELETE).0)
        .open(destination)?;
    if snapshot_from_file(&destination_guard)? != expected {
        return Err(file_changed_error());
    }

    let backup = unique_backup_path(destination)?;
    let backup_wide: Vec<u16> = backup.as_os_str().encode_wide().chain(Some(0)).collect();
    let replace_result = unsafe {
        ReplaceFileW(
            PCWSTR(target.as_ptr()),
            PCWSTR(source.as_ptr()),
            PCWSTR(backup_wide.as_ptr()),
            Default::default(),
            None,
            None,
        )
    };
    if let Err(error) = replace_result {
        drop(destination_guard);
        if !destination.exists()
            && capture_file_snapshot(&backup)
                .ok()
                .is_some_and(|actual| actual.same_file_identity(expected))
        {
            unsafe {
                MoveFileExW(
                    PCWSTR(backup_wide.as_ptr()),
                    PCWSTR(target.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(|restore_error| {
                io::Error::other(format!(
                    "Image replacement failed and the original remains at '{}': {}; {}",
                    backup.display(),
                    error,
                    restore_error
                ))
            })?;
        }
        return Err(io::Error::other(error.to_string()));
    }

    let backup_matches = capture_file_snapshot(&backup)
        .ok()
        .is_some_and(|actual| actual.same_file_identity(expected));
    if backup_matches {
        if let Err(error) = std::fs::remove_file(&backup) {
            log::warn!(
                "[IMAGE-VIEWER] cropped image published but backup cleanup failed: {}",
                error
            );
        }
        return Ok(());
    }

    drop(destination_guard);
    let conflict = unique_conflict_path(destination)?;
    let conflict_wide: Vec<u16> = conflict.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(target.as_ptr()),
            PCWSTR(conflict_wide.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| {
        io::Error::other(format!(
            "Destination changed during publication; preserved backup '{}': {}",
            backup.display(),
            error
        ))
    })?;
    let restore_result = unsafe {
        MoveFileExW(
            PCWSTR(backup_wide.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    match restore_result {
        Ok(()) => Err(file_changed_error()),
        Err(error) => Err(io::Error::other(format!(
            "Destination changed during publication; preserved files '{}' and '{}': {}",
            backup.display(),
            conflict.display(),
            error
        ))),
    }
}

fn unique_backup_path(destination: &Path) -> io::Result<PathBuf> {
    unique_sibling_path(destination, "backup")
}

#[cfg(target_os = "windows")]
fn unique_conflict_path(destination: &Path) -> io::Result<PathBuf> {
    unique_sibling_path(destination, "conflict")
}

fn unique_sibling_path(destination: &Path, kind: &str) -> io::Result<PathBuf> {
    let parent = destination.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Destination has no parent directory",
        )
    })?;
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    for _ in 0..100 {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!(
            ".{name}.mtt-crop-{kind}-{}-{sequence}.tmp",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Could not reserve a temporary backup path",
    ))
}

#[cfg(not(target_os = "windows"))]
fn publish_temporary(
    temporary: &Path,
    destination: &Path,
    replace_existing: bool,
    expected_destination: Option<FileSnapshot>,
) -> io::Result<()> {
    if !replace_existing {
        std::fs::hard_link(temporary, destination)?;
        if let Err(error) = std::fs::remove_file(temporary) {
            log::warn!(
                "[IMAGE-VIEWER] cropped image published but temporary cleanup failed: {}",
                error
            );
        }
        sync_parent_directory_best_effort(destination);
        return Ok(());
    }

    let expected = expected_destination.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Replacing a file requires a confirmed file snapshot",
        )
    })?;
    let backup = unique_backup_path(destination)?;
    std::fs::rename(destination, &backup)?;
    sync_parent_directory_best_effort(destination);

    if capture_file_snapshot(&backup).ok() != Some(expected) {
        restore_backup_if_destination_absent(&backup, destination)?;
        return Err(file_changed_error());
    }

    if let Err(publish_error) = std::fs::hard_link(temporary, destination) {
        let restore_result = restore_backup_if_destination_absent(&backup, destination);
        return match restore_result {
            Ok(()) => Err(publish_error),
            Err(restore_error) => Err(io::Error::other(format!(
                "Publishing failed ({publish_error}); original preserved at '{}' because restoration failed: {restore_error}",
                backup.display()
            ))),
        };
    }
    if let Err(error) = std::fs::remove_file(temporary) {
        log::warn!(
            "[IMAGE-VIEWER] cropped image published but temporary cleanup failed: {}",
            error
        );
    }
    if let Err(error) = std::fs::remove_file(&backup) {
        log::warn!(
            "[IMAGE-VIEWER] cropped image published but backup cleanup failed: {}",
            error
        );
    }
    sync_parent_directory_best_effort(destination);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn restore_backup_if_destination_absent(backup: &Path, destination: &Path) -> io::Result<()> {
    std::fs::hard_link(backup, destination)?;
    if let Err(error) = std::fs::remove_file(backup) {
        log::warn!(
            "[IMAGE-VIEWER] original restored but backup cleanup failed: {}",
            error
        );
    }
    sync_parent_directory_best_effort(destination);
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn sync_parent_directory_best_effort(path: &Path) {
    let result = path
        .parent()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Destination has no parent directory",
            )
        })
        .and_then(std::fs::File::open)
        .and_then(|directory| directory.sync_all());
    if let Err(error) = result {
        log::warn!(
            "[IMAGE-VIEWER] image was published but directory sync failed: {}",
            error
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> DecodedFrame {
        DecodedFrame {
            rgba: vec![255, 0, 0, 255],
            width: 1,
            height: 1,
            original_width: 1,
            original_height: 1,
        }
    }

    #[test]
    fn atomic_save_replaces_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"original").unwrap();

        let snapshot = capture_file_snapshot(&path).unwrap();
        save_frame_atomically(frame(), ExportImageFormat::Png, &path, true, Some(snapshot))
            .unwrap();

        let decoded = image::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 1));
        assert_eq!(decoded.to_rgba8().as_raw(), &[255, 0, 0, 255]);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn atomic_save_does_not_replace_without_permission() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"original").unwrap();

        assert!(
            save_frame_atomically(frame(), ExportImageFormat::Png, &path, false, None).is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn atomic_save_creates_new_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new-image.png");

        save_frame_atomically(frame(), ExportImageFormat::Png, &path, false, None).unwrap();

        let decoded = image::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1, 1));
        assert_eq!(decoded.to_rgba8().as_raw(), &[255, 0, 0, 255]);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn encode_failure_preserves_original_and_removes_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"original").unwrap();
        let invalid_frame = DecodedFrame {
            rgba: Vec::new(),
            width: 1,
            height: 1,
            original_width: 1,
            original_height: 1,
        };

        let snapshot = capture_file_snapshot(&path).unwrap();
        assert!(save_frame_atomically(
            invalid_frame,
            ExportImageFormat::Png,
            &path,
            true,
            Some(snapshot),
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn changed_destination_is_not_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"original").unwrap();
        let snapshot = capture_file_snapshot(&path).unwrap();
        std::fs::write(&path, b"newer contents").unwrap();

        assert!(save_frame_atomically(
            frame(),
            ExportImageFormat::Png,
            &path,
            true,
            Some(snapshot),
        )
        .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"newer contents");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn source_guard_blocks_writes_until_decode_finishes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        std::fs::write(&path, b"original").unwrap();
        let snapshot = capture_file_snapshot(&path).unwrap();

        let guard = guard_unchanged_source(&path, snapshot).unwrap();
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
        drop(guard);
        assert!(std::fs::OpenOptions::new().write(true).open(&path).is_ok());
    }
}
