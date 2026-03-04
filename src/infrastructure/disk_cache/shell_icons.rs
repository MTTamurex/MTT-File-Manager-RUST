//! Persistent SQLite cache for Windows shell icons (special folders, drives,
//! "This PC", Recycle Bin).
//!
//! Icons are stored as raw RGBA pixel data.  They are tiny (~256×256×4 ≈ 256 KB
//! each) so compression is unnecessary and avoids encode/decode overhead.

use super::ThumbnailDiskCache;
use rusqlite::params;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

impl ThumbnailDiskCache {
    // ── readers ──────────────────────────────────────────────────────────

    /// Retrieve a single cached shell icon by key.
    /// Returns `(rgba_pixels, width, height)` or `None` on cache miss.
    /// [READER]
    pub fn get_shell_icon(&self, key: &str) -> Option<(Vec<u8>, u32, u32)> {
        let db = self.reader.lock().ok()?;
        let mut stmt = db
            .prepare_cached("SELECT data, width, height FROM shell_icons WHERE key = ?")
            .ok()?;
        stmt.query_row([key], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, i64>(2)? as u32,
            ))
        })
        .ok()
        .filter(|(pixels, w, h)| {
            // Basic sanity check: pixel buffer must match dimensions.
            *w > 0 && *h > 0 && pixels.len() == (*w as usize) * (*h as usize) * 4
        })
    }

    /// Bulk-load all cached shell icons.
    /// Returns `HashMap<key, (rgba_pixels, width, height)>`.
    /// Typically completes in <5 ms for ~15 entries.
    /// [READER]
    pub fn get_all_shell_icons(&self) -> HashMap<String, (Vec<u8>, u32, u32)> {
        let mut map = HashMap::with_capacity(16);
        let Ok(db) = self.reader.lock() else {
            return map;
        };
        let Ok(mut stmt) = db.prepare_cached("SELECT key, data, width, height FROM shell_icons")
        else {
            return map;
        };
        let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)? as u32,
                row.get::<_, i64>(3)? as u32,
            ))
        }) else {
            return map;
        };
        for row in rows.flatten() {
            let (key, pixels, w, h) = row;
            if w > 0 && h > 0 && pixels.len() == (w as usize) * (h as usize) * 4 {
                map.insert(key, (pixels, w, h));
            }
        }
        map
    }

    // ── writers ──────────────────────────────────────────────────────────

    /// Persist a shell icon.  Uses INSERT OR REPLACE so callers can update
    /// icons that changed after a Windows theme switch.
    /// [WRITER]
    pub fn put_shell_icon(&self, key: &str, pixels: &[u8], width: u32, height: u32) {
        if key.is_empty() || pixels.is_empty() || width == 0 || height == 0 {
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let Ok(db) = self.writer.lock() else { return };
        let _ = db.execute(
            "INSERT OR REPLACE INTO shell_icons (key, data, width, height, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, pixels, width as i64, height as i64, now],
        );
    }

    /// Remove all cached shell icons (for manual invalidation).
    /// [WRITER]
    #[allow(dead_code)]
    pub fn clear_shell_icons(&self) {
        let Ok(db) = self.writer.lock() else { return };
        let _ = db.execute("DELETE FROM shell_icons", []);
    }
}
