use super::{ThumbnailCacheEntry, ThumbnailDiskCache};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

impl ThumbnailDiskCache {
    /// Folder covers must not reuse an image edited within the same timestamp second.
    pub fn get_folder_cover_thumbnail(
        &self,
        path: &Path,
        modified: SystemTime,
    ) -> Option<ThumbnailCacheEntry> {
        let modified = modified.duration_since(UNIX_EPOCH).ok()?;
        self.reader
            .lock()
            .query_row(
                "SELECT data, width, height, requested_size, modified_at FROM thumbnails
             WHERE id = ? AND modified_at = ? AND modified_nanos = ?",
                rusqlite::params![
                    Self::hash_path(path),
                    modified.as_secs() as i64,
                    modified.as_nanos().to_string()
                ],
                |row| {
                    Ok(ThumbnailCacheEntry {
                        data: row.get(0)?,
                        width: row.get::<_, i64>(1)? as u32,
                        height: row.get::<_, i64>(2)? as u32,
                        requested_size: row.get::<_, i64>(3)? as u32,
                        modified_at: row.get::<_, i64>(4)? as u64,
                    })
                },
            )
            .ok()
    }
}
