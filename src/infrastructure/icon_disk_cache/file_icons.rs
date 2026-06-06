use super::{expected_rgba_len, IconDiskCache};
use crate::domain::file_entry::IconSize;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_FILE_ICON_PNG_BYTES: u64 = 1024 * 1024;
const FILE_ICON_CACHE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;
const FILE_ICON_CACHE_TARGET_BYTES: u64 = 224 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct FileIconCacheKey {
    id: String,
    source_path: PathBuf,
    file_size: u64,
    modified_ns: u128,
    icon_size: IconSize,
}

struct FileIconEntry {
    id: String,
    source_path: PathBuf,
    byte_len: u64,
    last_accessed_at: i64,
}

pub(super) fn open_file_icon_db(app_data_dir: &Path) -> Connection {
    let db_path = app_data_dir.join("file_icons.db");
    if let Some(parent) = db_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            log::warn!(
                "[IconDiskCache] Failed to create file icon DB dir {:?}: {}",
                parent,
                error
            );
        }
    }

    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            log::warn!(
                "[IconDiskCache] Failed to open file icon DB {:?}: {}. Using in-memory fallback.",
                db_path,
                error
            );
            Connection::open_in_memory().expect("in-memory file icon cache should open")
        }
    };
    crate::infrastructure::db_utils::apply_default_pragmas(&conn);
    run_file_icon_migrations(&conn);
    conn
}

fn run_file_icon_migrations(conn: &Connection) {
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS file_icons (
            id TEXT PRIMARY KEY,
            source_path TEXT NOT NULL,
            data BLOB NOT NULL,
            width INTEGER NOT NULL,
            height INTEGER NOT NULL,
            icon_size TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            modified_ns TEXT NOT NULL,
            byte_len INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            last_accessed_at INTEGER NOT NULL
        )",
        [],
    );
    let _ = conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_file_icons_last_accessed \
         ON file_icons(last_accessed_at)",
        [],
    );
}

impl IconDiskCache {
    /// Build a stable cache key for a unique per-file icon.
    ///
    /// The key includes the path, file size, modification time, and icon size.
    /// If the executable/link changes, the key changes and stale pixels are not reused.
    pub fn file_icon_cache_key(
        &self,
        path: &Path,
        icon_size: IconSize,
    ) -> Option<FileIconCacheKey> {
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }

        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())?;

        let file_size = metadata.len();
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.as_os_str().as_encoded_bytes());
        hasher.update(&file_size.to_le_bytes());
        hasher.update(&modified_ns.to_le_bytes());
        hasher.update(icon_size_cache_tag(icon_size).as_bytes());
        let hash = hasher.finalize();

        Some(FileIconCacheKey {
            id: hash.to_hex()[..32].to_string(),
            source_path: path.to_path_buf(),
            file_size,
            modified_ns,
            icon_size,
        })
    }

    /// Load a unique per-file icon from the bounded lossless SQLite cache.
    pub fn load_file_icon(&self, key: &FileIconCacheKey) -> Option<(Vec<u8>, u32, u32)> {
        let (data, width, height): (Vec<u8>, u32, u32) = {
            let db = self.file_icon_db.lock();
            let mut stmt = db
                .prepare_cached("SELECT data, width, height FROM file_icons WHERE id = ?")
                .ok()?;
            stmt.query_row([&key.id], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, i64>(2)? as u32,
                ))
            })
            .ok()?
        };

        if data.len() as u64 > MAX_FILE_ICON_PNG_BYTES {
            self.delete_file_icon_rows(std::slice::from_ref(&key.id));
            return None;
        }

        let image = match image::load_from_memory_with_format(&data, ImageFormat::Png) {
            Ok(image) => image,
            Err(_) => {
                self.delete_file_icon_rows(std::slice::from_ref(&key.id));
                return None;
            }
        };
        let rgba = image.to_rgba8();
        if rgba.width() != width || rgba.height() != height {
            self.delete_file_icon_rows(std::slice::from_ref(&key.id));
            return None;
        }

        expected_rgba_len(width, height)?;
        self.touch_file_icon(&key.id);
        Some((rgba.to_vec(), width, height))
    }

    /// Save a unique per-file icon as lossless PNG blob in SQLite.
    pub fn save_file_icon(&self, key: &FileIconCacheKey, pixels: &[u8], width: u32, height: u32) {
        if pixels.is_empty() || expected_rgba_len(width, height) != Some(pixels.len()) {
            return;
        }

        let Some(encoded) = encode_png_lossless(pixels, width, height) else {
            return;
        };
        if encoded.len() as u64 > MAX_FILE_ICON_PNG_BYTES {
            return;
        }

        let now = current_epoch_secs();
        let inserted = {
            let db = self.file_icon_db.lock();
            db.execute(
                "INSERT OR IGNORE INTO file_icons
                 (id, source_path, data, width, height, icon_size, file_size, modified_ns,
                  byte_len, created_at, last_accessed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    key.id.as_str(),
                    key.source_path.to_string_lossy().as_ref(),
                    encoded.as_slice(),
                    width as i64,
                    height as i64,
                    icon_size_cache_tag(key.icon_size),
                    key.file_size as i64,
                    key.modified_ns.to_string(),
                    encoded.len() as i64,
                    now,
                    now,
                ],
            )
            .unwrap_or(0)
        };

        if inserted > 0 {
            let _ = self.trim_file_icon_cache_if_needed();
        }
    }

    /// Remove entries whose original file no longer exists and enforce the size cap.
    pub fn garbage_collect_file_icons(&self) -> usize {
        self.trim_file_icon_cache_if_needed()
    }

    fn trim_file_icon_cache_if_needed(&self) -> usize {
        let _guard = self.file_icon_trim_lock.lock();

        let mut entries = self.list_file_icon_entries();
        if entries.is_empty() {
            return 0;
        }

        let accessible = accessible_drives(
            entries
                .iter()
                .filter_map(|entry| extract_drive_root(&entry.source_path)),
        );

        let mut delete_ids: Vec<String> = entries
            .iter()
            .filter(|entry| {
                is_on_accessible_drive(&entry.source_path, &accessible)
                    && !crate::infrastructure::onedrive::fast_path_exists(&entry.source_path)
            })
            .map(|entry| entry.id.clone())
            .collect();

        let delete_set: HashSet<&str> = delete_ids.iter().map(String::as_str).collect();
        entries.retain(|entry| !delete_set.contains(entry.id.as_str()));

        let mut total: u64 = entries
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
                delete_ids.push(entry.id);
            }
        }

        self.delete_file_icon_rows(&delete_ids)
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

    fn delete_file_icon_rows(&self, ids: &[String]) -> usize {
        if ids.is_empty() {
            return 0;
        }
        let mut db = self.file_icon_db.lock();
        let Ok(tx) = db.transaction() else {
            return 0;
        };

        let mut removed = 0usize;
        {
            let Ok(mut stmt) = tx.prepare_cached("DELETE FROM file_icons WHERE id = ?") else {
                return 0;
            };
            for id in ids {
                removed += stmt.execute([id]).unwrap_or(0);
            }
        }
        if tx.commit().is_err() {
            return 0;
        }
        removed
    }

    fn touch_file_icon(&self, id: &str) {
        let db = self.file_icon_db.lock();
        let _ = db.execute(
            "UPDATE file_icons SET last_accessed_at = ?1 WHERE id = ?2",
            params![current_epoch_secs(), id],
        );
    }
}

fn current_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
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
        let is_accessible = *checked
            .entry(root.clone())
            .or_insert_with(|| crate::infrastructure::onedrive::fast_path_exists(Path::new(&root)));
        if is_accessible {
            accessible.insert(root);
        }
    }

    accessible
}

fn is_on_accessible_drive(path: &Path, accessible: &HashSet<String>) -> bool {
    extract_drive_root(path).is_none_or(|root| accessible.contains(&root))
}

fn icon_size_cache_tag(size: IconSize) -> &'static str {
    match size {
        IconSize::Small => "small",
        IconSize::Large => "large",
        IconSize::Jumbo => "jumbo",
    }
}

fn encode_png_lossless(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, pixels.to_vec())?;
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut cursor, ImageFormat::Png)
        .ok()?;
    Some(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_icon_cache_round_trips_lossless_png() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let cache = IconDiskCache::new(dir.path());
        let exe = dir.path().join("tool.exe");
        std::fs::write(&exe, b"v1").expect("write source file");

        let key = cache
            .file_icon_cache_key(&exe, IconSize::Jumbo)
            .expect("file icon key");
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
        ];

        cache.save_file_icon(&key, &pixels, 2, 2);

        let (loaded, width, height) = cache.load_file_icon(&key).expect("cached icon");
        assert_eq!(loaded, pixels);
        assert_eq!(width, 2);
        assert_eq!(height, 2);
    }

    #[test]
    fn file_icon_cache_key_changes_when_file_size_changes() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let cache = IconDiskCache::new(dir.path());
        let exe = dir.path().join("tool.exe");
        std::fs::write(&exe, b"v1").expect("write first version");
        let key_v1 = cache
            .file_icon_cache_key(&exe, IconSize::Jumbo)
            .expect("first key");

        std::fs::write(&exe, b"version two").expect("write second version");
        let key_v2 = cache
            .file_icon_cache_key(&exe, IconSize::Jumbo)
            .expect("second key");

        assert_ne!(key_v1, key_v2);
    }

    #[test]
    fn file_icon_cache_removes_orphaned_source_files_during_scan() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let cache = IconDiskCache::new(dir.path());
        let exe = dir.path().join("tool.exe");
        std::fs::write(&exe, b"v1").expect("write source file");
        let key = cache
            .file_icon_cache_key(&exe, IconSize::Jumbo)
            .expect("file icon key");
        let pixels = vec![
            255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 64, 255, 255, 255, 0,
        ];

        cache.save_file_icon(&key, &pixels, 2, 2);
        assert!(cache.load_file_icon(&key).is_some());

        std::fs::remove_file(&exe).expect("delete source file");
        assert_eq!(cache.garbage_collect_file_icons(), 1);

        assert!(cache.load_file_icon(&key).is_none());
    }
}
