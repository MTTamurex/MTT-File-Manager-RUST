use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::domain::file_entry::{is_path_inside_archive, FileEntry};
use crate::domain::special_paths::{tag_id_from_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};

pub type FileHashRequest = (PathBuf, u64, u64);
pub type FileHashResponse = (PathBuf, u64, u64, Result<String, String>);
pub type SelectedFileHash = (PathBuf, u64, u64, Result<String, String>);

pub const FILE_HASH_READ_CHUNK: usize = 256 * 1024;

#[derive(Clone, Copy)]
pub struct FileHashStatus {
    pub modified: u64,
    pub size: u64,
}

fn is_hashable_file_entry(file: &FileEntry) -> bool {
    !file.is_dir || (file.is_archive() && (file.size > 0 || file.path().is_file()))
}

pub fn can_hash_file(file: &FileEntry) -> bool {
    is_hashable_file_entry(file)
        && file.drive_info.is_none()
        && file.name != COMPUTER_VIEW_ID
        && file.name != RECYCLE_BIN_VIEW_ID
        && !file.is_recycle_item()
        && !is_path_inside_archive(file.path())
        && tag_id_from_view_path(&file.path.to_string_lossy()).is_none()
}

pub fn file_hash_status(file: &FileEntry) -> FileHashStatus {
    let size = if file.is_dir && file.is_archive() && file.size == 0 {
        file.path()
            .metadata()
            .ok()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .unwrap_or(file.size)
    } else {
        file.size
    };

    FileHashStatus {
        modified: file.modified,
        size,
    }
}

pub fn selected_hash_result(
    selected: &Option<SelectedFileHash>,
    path: &Path,
    status: FileHashStatus,
) -> Option<Result<String, String>> {
    selected
        .as_ref()
        .and_then(|(cached_path, modified, size, result)| {
            if cached_path == path && *modified == status.modified && *size == status.size {
                Some(result.clone())
            } else {
                None
            }
        })
}

pub fn compute_sha256_streaming(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path).map_err(|e| format!("open: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; FILE_HASH_READ_CHUNK];
    loop {
        let n = file.read(&mut buffer).map_err(|e| format!("read: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    let digest = hasher.finalize();
    Ok(format!("{:x}", digest))
}

pub fn try_enqueue_file_hash<S>(
    path: &Path,
    status: FileHashStatus,
    loading: &mut std::collections::HashSet<PathBuf, S>,
    request_sender: &mpsc::Sender<FileHashRequest>,
) -> bool
where
    S: std::hash::BuildHasher,
{
    if loading.contains(path) {
        return false;
    }
    if request_sender
        .send((path.to_path_buf(), status.modified, status.size))
        .is_ok()
    {
        loading.insert(path.to_path_buf());
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{can_hash_file, file_hash_status};
    use crate::domain::file_entry::FileEntry;

    #[test]
    fn allows_archive_files_marked_as_directories() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("sample.7z");
        std::fs::write(&archive, b"archive bytes").expect("create archive file");
        let entry = FileEntry::from_path(archive, true);

        assert!(can_hash_file(&entry));
        assert_eq!(file_hash_status(&entry).size, 13);
    }

    #[test]
    fn rejects_real_directories_named_like_archives() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive_named_dir = dir.path().join("sample.zip");
        std::fs::create_dir(&archive_named_dir).expect("create archive-named dir");
        let entry = FileEntry::from_path(archive_named_dir, true);

        assert!(!can_hash_file(&entry));
    }

    #[test]
    fn hash_status_reports_real_size_for_archive_marked_as_directory() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let archive = dir.path().join("sample.zip");
        std::fs::write(&archive, vec![0u8; 4096]).expect("create archive file");
        let mut entry = FileEntry::from_path(archive, true);
        entry.size = 0;

        let status = file_hash_status(&entry);
        assert_eq!(status.size, 4096);
    }
}
