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

pub fn can_hash_file(file: &FileEntry) -> bool {
    !file.is_dir
        && file.drive_info.is_none()
        && file.name != COMPUTER_VIEW_ID
        && file.name != RECYCLE_BIN_VIEW_ID
        && !file.is_recycle_item()
        && !is_path_inside_archive(file.path())
        && tag_id_from_view_path(&file.path.to_string_lossy()).is_none()
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
