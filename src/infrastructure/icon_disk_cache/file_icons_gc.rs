use super::file_icons::{
    file_icon_cache_bytes, FileIconEntry, FILE_ICON_CACHE_LIMIT_BYTES, FILE_ICON_CACHE_TARGET_BYTES,
};
use super::IconDiskCache;
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

impl IconDiskCache {
    /// Remove entries whose original file no longer exists and enforce the size cap.
    pub fn garbage_collect_file_icons(&self) -> usize {
        self.trim_file_icon_cache_if_needed()
    }

    fn trim_file_icon_cache_if_needed(&self) -> usize {
        let mut entries = self.list_file_icon_entries();
        if entries.is_empty() {
            return 0;
        }

        let accessible = accessible_drives(
            entries
                .iter()
                .filter_map(|entry| extract_drive_root(&entry.source_path)),
        );
        let mut delete_entries: Vec<(String, i64)> = entries
            .iter()
            .filter(|entry| {
                is_on_accessible_drive(&entry.source_path, &accessible)
                    && !crate::infrastructure::onedrive::is_cloud_sync_path(&entry.source_path)
                    && source_is_definitely_missing(&entry.source_path)
            })
            .map(|entry| (entry.id.clone(), entry.last_accessed_at))
            .collect();
        let delete_set: HashSet<&str> = delete_entries.iter().map(|(id, _)| id.as_str()).collect();
        entries.retain(|entry| !delete_set.contains(entry.id.as_str()));

        // Filesystem checks can be slow on external or cloud paths. Serialize
        // only the database eviction phase with size-based trimming.
        let _guard = self.file_icon_trim_lock.lock();
        let mut total = entries
            .iter()
            .map(|entry| entry.byte_len)
            .fold(0u64, u64::saturating_add);
        if total > FILE_ICON_CACHE_LIMIT_BYTES {
            entries.sort_by_key(|entry| entry.last_accessed_at);
            for entry in entries {
                if total <= FILE_ICON_CACHE_TARGET_BYTES {
                    break;
                }
                total = total.saturating_sub(entry.byte_len);
                delete_entries.push((entry.id, entry.last_accessed_at));
            }
        }

        if delete_entries.is_empty() {
            self.sync_file_icon_cache_bytes();
            0
        } else {
            self.delete_file_icon_rows(&delete_entries)
        }
    }

    pub(super) fn trim_file_icon_cache_size_if_needed(&self) -> usize {
        if self.file_icon_cache_bytes.load(Ordering::Relaxed) <= FILE_ICON_CACHE_LIMIT_BYTES {
            return 0;
        }

        let _guard = self.file_icon_trim_lock.lock();
        let mut entries = self.list_file_icon_entries();
        let mut total = entries
            .iter()
            .map(|entry| entry.byte_len)
            .fold(0u64, u64::saturating_add);
        if total <= FILE_ICON_CACHE_LIMIT_BYTES {
            self.sync_file_icon_cache_bytes();
            return 0;
        }

        entries.sort_by_key(|entry| entry.last_accessed_at);
        let mut delete_entries = Vec::new();
        for entry in entries {
            if total <= FILE_ICON_CACHE_TARGET_BYTES {
                break;
            }
            total = total.saturating_sub(entry.byte_len);
            delete_entries.push((entry.id, entry.last_accessed_at));
        }
        self.delete_file_icon_rows(&delete_entries)
    }

    fn list_file_icon_entries(&self) -> Vec<FileIconEntry> {
        let db = self.file_icon_db.lock();
        let Ok(mut stmt) =
            db.prepare_cached("SELECT id, source_path, byte_len, last_accessed_at FROM file_icons")
        else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok(FileIconEntry {
                id: row.get::<_, String>(0)?,
                source_path: PathBuf::from(row.get::<_, String>(1)?),
                byte_len: row.get::<_, i64>(2)?.max(0) as u64,
                last_accessed_at: row.get::<_, i64>(3)?,
            })
        }) else {
            return Vec::new();
        };
        rows.flatten().collect()
    }

    fn delete_file_icon_rows(&self, entries: &[(String, i64)]) -> usize {
        if entries.is_empty() {
            return 0;
        }
        let mut db = self.file_icon_db.lock();
        let tx = match db.transaction() {
            Ok(tx) => tx,
            Err(error) => {
                log::warn!(
                    "[IconDiskCache] Failed to start delete transaction: {}",
                    error
                );
                return 0;
            }
        };
        let mut removed = 0usize;
        {
            let mut stmt = match tx
                .prepare_cached("DELETE FROM file_icons WHERE id = ?1 AND last_accessed_at = ?2")
            {
                Ok(stmt) => stmt,
                Err(error) => {
                    log::warn!("[IconDiskCache] Failed to prepare icon deletion: {}", error);
                    return 0;
                }
            };
            for (id, last_accessed_at) in entries {
                match stmt.execute(params![id, last_accessed_at]) {
                    Ok(count) => removed += count,
                    Err(error) => {
                        log::warn!("[IconDiskCache] Failed to delete icon row: {}", error)
                    }
                }
            }
        }
        if let Err(error) = tx.commit() {
            log::warn!("[IconDiskCache] Failed to commit icon deletion: {}", error);
            return 0;
        }
        match file_icon_cache_bytes(&db) {
            Ok(total) => self.file_icon_cache_bytes.store(total, Ordering::Relaxed),
            Err(error) => log::warn!("[IconDiskCache] Failed to refresh cache size: {}", error),
        }
        removed
    }

    pub(super) fn sync_file_icon_cache_bytes(&self) {
        let db = self.file_icon_db.lock();
        match file_icon_cache_bytes(&db) {
            Ok(total) => self.file_icon_cache_bytes.store(total, Ordering::Relaxed),
            Err(error) => log::warn!("[IconDiskCache] Failed to refresh cache size: {}", error),
        }
    }
}

fn extract_drive_root(path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    if path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
        && (path.as_bytes()[2] == b'\\' || path.as_bytes()[2] == b'/')
    {
        Some(format!("{}:\\", path.as_bytes()[0] as char))
    } else {
        None
    }
}

fn accessible_drives(roots: impl Iterator<Item = String>) -> HashSet<String> {
    let mut checked: HashMap<String, bool> = HashMap::new();
    let mut accessible = HashSet::new();
    for root in roots {
        if crate::infrastructure::windows::detect_drive_type(&root)
            != crate::infrastructure::windows::DriveType::Fixed
        {
            continue;
        }
        let is_accessible = *checked
            .entry(root.clone())
            .or_insert_with(|| crate::infrastructure::onedrive::fast_path_exists(Path::new(&root)));
        if is_accessible {
            accessible.insert(root);
        }
    }
    accessible
}

pub(super) fn is_on_accessible_drive(path: &Path, accessible: &HashSet<String>) -> bool {
    extract_drive_root(path).is_some_and(|root| accessible.contains(&root))
}

fn source_is_definitely_missing(path: &Path) -> bool {
    use windows::Win32::Foundation::{GetLastError, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attrs = unsafe { GetFileAttributesW(windows::core::PCWSTR(path_wide.as_ptr())) };
    if attrs != INVALID_FILE_ATTRIBUTES {
        return false;
    }
    let error = unsafe { GetLastError() };
    error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND
}
