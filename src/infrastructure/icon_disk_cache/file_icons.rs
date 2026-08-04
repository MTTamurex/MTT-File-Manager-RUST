use super::{expected_rgba_len, IconDiskCache};
use crate::domain::file_entry::IconSize;
use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use rusqlite::{params, Connection};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_FILE_ICON_PNG_BYTES: u64 = 1024 * 1024;
pub(super) const FILE_ICON_CACHE_LIMIT_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const FILE_ICON_CACHE_TARGET_BYTES: u64 = 224 * 1024 * 1024;
const FILE_ICON_TOUCH_INTERVAL_SECS: i64 = 60 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct FileIconCacheKey {
    pub(super) id: String,
    source_path: PathBuf,
    file_size: u64,
    modified_ns: u128,
    icon_size: IconSize,
}

pub(super) struct FileIconEntry {
    pub(super) id: String,
    pub(super) source_path: PathBuf,
    pub(super) byte_len: u64,
    pub(super) last_accessed_at: i64,
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

pub(super) fn run_file_icon_migrations(conn: &Connection) {
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

pub(super) fn file_icon_cache_bytes(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COALESCE(SUM(byte_len), 0) FROM file_icons",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|total| total.max(0) as u64)
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
        let (data, width_raw, height_raw, last_accessed_at): (Vec<u8>, i64, i64, i64) = {
            let db = self.file_icon_db.lock();
            let mut stmt = db
                .prepare_cached(
                    "SELECT data, width, height, last_accessed_at FROM file_icons WHERE id = ?",
                )
                .ok()?;
            stmt.query_row([&key.id], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .ok()?
        };

        let (width, height) = match (u32::try_from(width_raw), u32::try_from(height_raw)) {
            (Ok(width), Ok(height)) => (width, height),
            _ => {
                self.delete_file_icon_if_unchanged(
                    &key.id,
                    &data,
                    width_raw,
                    height_raw,
                    last_accessed_at,
                );
                return None;
            }
        };

        if data.len() as u64 > MAX_FILE_ICON_PNG_BYTES {
            self.delete_file_icon_if_unchanged(
                &key.id,
                &data,
                width_raw,
                height_raw,
                last_accessed_at,
            );
            return None;
        }

        if expected_rgba_len(width, height).is_none() {
            self.delete_file_icon_if_unchanged(
                &key.id,
                &data,
                width_raw,
                height_raw,
                last_accessed_at,
            );
            return None;
        }
        let dimensions =
            image::ImageReader::with_format(Cursor::new(data.as_slice()), ImageFormat::Png)
                .into_dimensions();
        if !matches!(dimensions, Ok((png_width, png_height)) if png_width == width && png_height == height)
        {
            self.delete_file_icon_if_unchanged(
                &key.id,
                &data,
                width_raw,
                height_raw,
                last_accessed_at,
            );
            return None;
        }

        let image = match image::load_from_memory_with_format(&data, ImageFormat::Png) {
            Ok(image) => image,
            Err(_) => {
                self.delete_file_icon_if_unchanged(
                    &key.id,
                    &data,
                    width_raw,
                    height_raw,
                    last_accessed_at,
                );
                return None;
            }
        };
        let rgba = image.to_rgba8();
        if rgba.width() != width || rgba.height() != height {
            self.delete_file_icon_if_unchanged(
                &key.id,
                &data,
                width_raw,
                height_raw,
                last_accessed_at,
            );
            return None;
        }
        let now = current_epoch_secs();
        if now.saturating_sub(last_accessed_at) >= FILE_ICON_TOUCH_INTERVAL_SECS {
            self.touch_file_icon(&key.id, now);
        }
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
        let total_after_save = {
            let db = self.file_icon_db.lock();
            let write_result = db.execute(
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
            );
            match write_result {
                Ok(0) => return,
                Ok(_) => {
                    self.file_icon_cache_bytes
                        .fetch_add(encoded.len() as u64, Ordering::Relaxed);
                }
                Err(error) => {
                    log::warn!("[IconDiskCache] Failed to persist icon: {}", error);
                    return;
                }
            }
            self.file_icon_cache_bytes.load(Ordering::Relaxed)
        };

        if total_after_save > FILE_ICON_CACHE_LIMIT_BYTES {
            let _ = self.trim_file_icon_cache_size_if_needed();
        }
    }

    pub(super) fn delete_file_icon_if_unchanged(
        &self,
        id: &str,
        expected_data: &[u8],
        expected_width: i64,
        expected_height: i64,
        expected_last_accessed_at: i64,
    ) {
        let db = self.file_icon_db.lock();
        match db.execute(
            "DELETE FROM file_icons
             WHERE id = ?1 AND data = ?2 AND width = ?3 AND height = ?4
               AND last_accessed_at = ?5",
            params![
                id,
                expected_data,
                expected_width,
                expected_height,
                expected_last_accessed_at
            ],
        ) {
            Ok(removed) if removed > 0 => match file_icon_cache_bytes(&db) {
                Ok(total) => self.file_icon_cache_bytes.store(total, Ordering::Relaxed),
                Err(error) => {
                    log::warn!("[IconDiskCache] Failed to refresh cache size: {}", error)
                }
            },
            Ok(_) => {}
            Err(error) => log::warn!("[IconDiskCache] Failed to delete corrupt icon: {}", error),
        }
    }

    fn touch_file_icon(&self, id: &str, now: i64) {
        let db = self.file_icon_db.lock();
        let _ = db.execute(
            "UPDATE file_icons SET last_accessed_at = ?1 WHERE id = ?2",
            params![now, id],
        );
    }
}

fn current_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn icon_size_cache_tag(size: IconSize) -> &'static str {
    match size {
        IconSize::Small => "small",
        IconSize::Large => "large",
        IconSize::Jumbo => "jumbo",
    }
}

pub(super) fn encode_png_lossless(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, pixels.to_vec())?;
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(img)
        .write_to(&mut cursor, ImageFormat::Png)
        .ok()?;
    Some(cursor.into_inner())
}
