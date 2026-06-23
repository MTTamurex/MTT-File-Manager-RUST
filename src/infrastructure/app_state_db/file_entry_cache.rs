//! Persistent cache of file metadata used by tag view loading.
//!
//! Tag views (e.g. "all files tagged Red") span many directories, so listing
//! them requires one `GetFileAttributesExW` syscall per file. On a cold NTFS
//! cache (after restart or long idle) each syscall is 5-15ms on HDD, and
//! there is no spatial locality between files of the same tag. This module
//! stores the metadata in a SQLite table so that subsequent tag selections
//! can serve from the cache without touching the disk at all.
//!
//! ## Invalidation
//!
//! The cache is treated as a hint. Entries are always revalidated by
//! callers through `DropWatcher`/`DriveWatcher` events (`Modified`,
//! `Deleted`, `Renamed`, `PrefixInvalidated`, `DriveLost`) — see
//! `crate::app::operations::message_handler::watcher_events` for the hook.
//!
//! OneDrive cloud paths are intentionally NOT cached: their `sync_status`
//! changes frequently (recall / hydration) and a stale value would be
//! misleading in the UI.

use super::AppStateDb;
use crate::domain::file_entry::{FileEntry, SyncStatus};
use rusqlite::params;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

const STORAGE_CHUNK_SIZE: usize = 500;

fn normalize_path_text(path: &str) -> String {
    path.replace('/', "\\").trim_end_matches('\\').to_string()
}

fn storage_path_text(path: &Path) -> Option<String> {
    path.to_str().map(normalize_path_text)
}

fn decode_sync_status(value: i64) -> SyncStatus {
    // Mirror the variants defined in `domain::file_entry::SyncStatus`. The
    // cloud-only path is the most common non-`None` state, and the cache
    // never stores OneDrive paths (filtered at insert/read time) so the
    // remaining variants are usually `None`.
    match value {
        0 => SyncStatus::None,
        1 => SyncStatus::CloudOnly,
        2 => SyncStatus::Syncing,
        3 => SyncStatus::Pinned,
        4 => SyncStatus::LocallyAvailable,
        _ => SyncStatus::None,
    }
}

fn encode_sync_status(status: SyncStatus) -> i64 {
    match status {
        SyncStatus::None => 0,
        SyncStatus::CloudOnly => 1,
        SyncStatus::Syncing => 2,
        SyncStatus::Pinned => 3,
        SyncStatus::LocallyAvailable => 4,
    }
}

fn row_from_entry(entry: &FileEntry) -> (String, i64, i64, i64, Option<i64>, i64, i64, i64) {
    let path = storage_path_text(&entry.path).unwrap_or_default();
    let cached_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    (
        path,
        if entry.is_dir { 1 } else { 0 },
        entry.size as i64,
        entry.modified as i64,
        entry.created.map(|c| c as i64),
        if entry.is_hidden { 1 } else { 0 },
        encode_sync_status(entry.sync_status),
        cached_at,
    )
}

impl AppStateDb {
    /// Look up cached metadata for a set of paths in a single query.
    /// Returns a map keyed by `PathBuf` (the original casing from the input)
    /// containing only the paths that have a cache entry.
    ///
    /// OneDrive cloud paths are filtered out from the result — the caller
    /// must re-stat those.
    pub fn get_cached_file_entries(
        &self,
        paths: &[PathBuf],
    ) -> FxHashMap<PathBuf, FileEntry> {
        if paths.is_empty() {
            return FxHashMap::default();
        }
        // Keys into the lookup map are lowercased so the post-query join is
        // case-insensitive. The SQLite table uses COLLATE NOCASE on its
        // primary key, but the in-memory `by_path` HashMap is case-sensitive
        // by default — without lowercasing, an input path whose casing
        // differs from the row stored in the DB would miss the join and the
        // caller would re-stat the file unnecessarily.
        let mut by_path: FxHashMap<String, PathBuf> = FxHashMap::default();
        for p in paths {
            if let Some(text) = storage_path_text(p) {
                by_path
                    .entry(text.to_lowercase())
                    .or_insert_with(|| p.clone());
            }
        }
        if by_path.is_empty() {
            return FxHashMap::default();
        }

        let db = match self.reader.lock() {
            Ok(db) => db,
            Err(e) => {
                log::warn!(
                    "[FILE-ENTRY-CACHE] Failed to acquire reader lock: {:?}",
                    e
                );
                return FxHashMap::default();
            }
        };

        let keys: Vec<String> = by_path.keys().cloned().collect();
        let mut result: FxHashMap<PathBuf, FileEntry> = FxHashMap::default();

        for chunk in keys.chunks(STORAGE_CHUNK_SIZE) {
            let placeholders =
                std::iter::repeat_n("?", chunk.len()).collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT file_path, is_dir, size, modified, created, is_hidden, sync_status \
                 FROM file_entry_cache WHERE file_path COLLATE NOCASE IN ({})",
                placeholders
            );
            let params_iter: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let Ok(mut stmt) = db.prepare(&sql) else {
                continue;
            };
            let Ok(rows) = stmt.query_map(params_iter.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            }) else {
                continue;
            };

            for row in rows {
                let (raw_path, is_dir, size, modified, created, is_hidden, sync_status) =
                    match row {
                        Ok(r) => r,
                        Err(_) => continue,
                    };
                let key = normalize_path_text(&raw_path).to_lowercase();
                let Some(original) = by_path.get(&key) else {
                    continue;
                };
                // Skip OneDrive cloud paths: sync_status is too volatile to
                // serve from cache.
                if crate::infrastructure::onedrive::is_cloud_sync_path(original) {
                    continue;
                }
                let entry = FileEntry {
                    path: original.clone(),
                    name: original
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    is_dir: is_dir != 0,
                    size: size.max(0) as u64,
                    modified: modified.max(0) as u64,
                    created: created.map(|c| c.max(0) as u64),
                    folder_cover: None,
                    drive_info: None,
                    sync_status: decode_sync_status(sync_status),
                    is_hidden: is_hidden != 0,
                    recycle_bin: None,
                };
                result.insert(original.clone(), entry);
            }
        }

        result
    }

    /// Persist (or refresh) entries in the cache. Existing rows are
    /// overwritten. OneDrive cloud paths are ignored — the cache only
    /// contains local-disk metadata.
    pub fn upsert_cached_file_entries(&self, entries: &[FileEntry]) {
        if entries.is_empty() {
            return;
        }
        let mut local: Vec<FileEntry> = entries
            .iter()
            .filter(|e| !crate::infrastructure::onedrive::is_cloud_sync_path(&e.path))
            .cloned()
            .collect();
        if local.is_empty() {
            return;
        }
        let db = match self.writer.lock() {
            Ok(db) => db,
            Err(e) => {
                log::warn!(
                    "[FILE-ENTRY-CACHE] Failed to acquire writer lock: {:?}",
                    e
                );
                return;
            }
        };

        let tx = match db.unchecked_transaction() {
            Ok(tx) => tx,
            Err(e) => {
                log::warn!("[FILE-ENTRY-CACHE] Failed to start transaction: {:?}", e);
                return;
            }
        };

        for entry in local.drain(..) {
            let (path, is_dir, size, modified, created, is_hidden, sync_status, cached_at) =
                row_from_entry(&entry);
            if path.is_empty() {
                continue;
            }
            let _ = tx.execute(
                "INSERT OR REPLACE INTO file_entry_cache \
                 (file_path, is_dir, size, modified, created, is_hidden, sync_status, cached_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    path,
                    is_dir,
                    size,
                    modified,
                    created,
                    is_hidden,
                    sync_status,
                    cached_at
                ],
            );
        }

        let _ = tx.commit();
    }

    /// Invalidate one path. Called from `DriveWatcher` Modified/Deleted events.
    pub fn invalidate_cached_file_entry(&self, path: &Path) {
        let Some(text) = storage_path_text(path) else {
            return;
        };
        let db = match self.writer.lock() {
            Ok(db) => db,
            Err(_) => return,
        };
        let _ = db.execute(
            "DELETE FROM file_entry_cache WHERE file_path = ?1 COLLATE NOCASE",
            params![text],
        );
    }

    /// Invalidate all cache entries under a path prefix. Called from
    /// `DriveWatcher` PrefixInvalidated / DriveLost.
    pub fn invalidate_cached_file_entries_under(&self, prefix: &Path) {
        let Some(prefix_text) = storage_path_text(prefix) else {
            return;
        };
        let db = match self.writer.lock() {
            Ok(db) => db,
            Err(_) => return,
        };
        let pattern = format!("{}%", prefix_text);
        let _ = db.execute(
            "DELETE FROM file_entry_cache WHERE file_path LIKE ?1 COLLATE NOCASE \
             ESCAPE '\\'",
            params![escape_like(&pattern)],
        );
    }

    /// Move a cache entry to a new path. Called from `DriveWatcher` Renamed.
    pub fn rename_cached_file_entry(&self, old_path: &Path, new_path: &Path) {
        let (Some(old_text), Some(new_text)) =
            (storage_path_text(old_path), storage_path_text(new_path))
        else {
            return;
        };
        let db = match self.writer.lock() {
            Ok(db) => db,
            Err(_) => return,
        };
        let _ = db.execute(
            "UPDATE file_entry_cache SET file_path = ?1 WHERE file_path = ?2 COLLATE NOCASE",
            params![new_text, old_text],
        );
    }
}

/// Escape `%` and `_` in a string for use with a SQLite `LIKE` pattern.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::{FileEntry, SyncStatus};
    use std::path::PathBuf;

    fn sample_entry(path: &str) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            name: path.to_string(),
            is_dir: false,
            size: 1234,
            modified: 1_700_000_000,
            created: Some(1_600_000_000),
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn row_from_entry_round_trips_basic_fields() {
        let entry = sample_entry(r"C:\foo\bar.txt");
        let (path, is_dir, size, modified, created, is_hidden, _sync, _cached) =
            row_from_entry(&entry);
        assert_eq!(path, r"C:\foo\bar.txt");
        assert_eq!(is_dir, 0);
        assert_eq!(size, 1234);
        assert_eq!(modified, 1_700_000_000);
        assert_eq!(created, Some(1_600_000_000));
        assert_eq!(is_hidden, 0);
    }

    #[test]
    fn normalize_path_text_replaces_slashes_and_strips_trailing_sep() {
        assert_eq!(normalize_path_text("C:/foo/bar/"), r"C:\foo\bar");
        assert_eq!(normalize_path_text("C:/foo/bar"), r"C:\foo\bar");
    }

    #[test]
    fn escape_like_handles_specials() {
        assert_eq!(escape_like("C:\\foo%bar"), "C:\\\\foo\\%bar");
        assert_eq!(escape_like("C:\\foo_bar"), "C:\\\\foo\\_bar");
    }

    /// Smoke test the SQLite-backed cache round-trip and case-insensitive
    /// lookup (the bug that motivated the lowercasing fix in
    /// `get_cached_file_entries`).
    #[test]
    fn app_state_db_file_entry_cache_roundtrip_case_insensitive() {
        use crate::infrastructure::app_state_db::AppStateDb;
        let tmp = std::env::temp_dir().join(format!(
            "mtt_file_entry_cache_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let db = AppStateDb::new(tmp.clone()).expect("open AppStateDb");

        // Insert an entry stored with mixed casing.
        let stored = sample_entry(r"C:\Foo\Bar.txt");
        db.upsert_cached_file_entries(&[stored]);

        // Lookup with the SAME casing must hit.
        let hit_same = db.get_cached_file_entries(&[PathBuf::from(r"C:\Foo\Bar.txt")]);
        assert_eq!(hit_same.len(), 1);
        assert_eq!(hit_same.values().next().unwrap().size, 1234);

        // Lookup with DIFFERENT casing must also hit — this is the bug the
        // lowercased-to_lowercase keys fix.
        let hit_lower = db.get_cached_file_entries(&[PathBuf::from(r"C:\foo\bar.txt")]);
        assert_eq!(
            hit_lower.len(),
            1,
            "case-insensitive lookup failed: cache should hit on different casing"
        );
        assert_eq!(hit_lower.values().next().unwrap().size, 1234);

        // Invalidate via different casing — should remove the row.
        db.invalidate_cached_file_entry(std::path::Path::new(r"C:\FOO\BAR.TXT"));
        let after = db.get_cached_file_entries(&[PathBuf::from(r"C:\Foo\Bar.txt")]);
        assert!(after.is_empty(), "invalidate did not remove the row");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Validate that `rename_cached_file_entry` moves the row case-insensitively.
    #[test]
    fn app_state_db_file_entry_cache_rename_case_insensitive() {
        use crate::infrastructure::app_state_db::AppStateDb;
        let tmp = std::env::temp_dir().join(format!(
            "mtt_file_entry_cache_rename_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let db = AppStateDb::new(tmp.clone()).expect("open AppStateDb");

        db.upsert_cached_file_entries(&[sample_entry(r"C:\Folder\old.txt")]);
        db.rename_cached_file_entry(
            std::path::Path::new(r"C:\folder\OLD.TXT"),
            std::path::Path::new(r"C:\folder\new.txt"),
        );

        let hits = db.get_cached_file_entries(&[
            PathBuf::from(r"C:\Folder\old.txt"),
            PathBuf::from(r"C:\folder\new.txt"),
        ]);
        assert!(!hits.contains_key(&PathBuf::from(r"C:\Folder\old.txt")));
        assert!(hits.contains_key(&PathBuf::from(r"C:\folder\new.txt")));
        assert_eq!(
            hits.get(&PathBuf::from(r"C:\folder\new.txt"))
                .unwrap()
                .size,
            1234
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
