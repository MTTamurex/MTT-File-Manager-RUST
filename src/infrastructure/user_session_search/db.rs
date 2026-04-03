use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use super::{IndexedItem, IndexedVolume};
use super::scanner::normalize_path_key;

pub(super) fn open_session_db() -> Option<rusqlite::Connection> {
    let cache_dir = dirs::data_local_dir()?
        .join("MTT-File-Manager")
        .join("thumbnails");
    let db_path = cache_dir.join("session_search.db");

    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        log::warn!(
            "[SESSION-SEARCH] Failed to create cache dir {:?}: {}",
            cache_dir,
            e
        );
        return None;
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("[SESSION-SEARCH] Failed to open session_search.db: {}", e);
            return None;
        }
    };

    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");

    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_items (
            drive_letter TEXT NOT NULL,
            name         TEXT NOT NULL,
            full_path    TEXT NOT NULL,
            is_dir       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_items_drive
            ON session_items(drive_letter);
        CREATE TABLE IF NOT EXISTS session_volumes (
            drive_letter  TEXT PRIMARY KEY,
            label         TEXT NOT NULL,
            file_system   TEXT NOT NULL
        );",
    ) {
        log::warn!("[SESSION-SEARCH] Failed to create session tables: {}", e);
        return None;
    }

    Some(conn)
}

pub(super) fn load_all_volumes(conn: &rusqlite::Connection) -> HashMap<char, IndexedVolume> {
    let mut volumes = HashMap::new();

    let mut vol_stmt =
        match conn.prepare("SELECT drive_letter, label, file_system FROM session_volumes") {
            Ok(s) => s,
            Err(_) => return volumes,
        };

    let vol_rows = match vol_stmt.query_map([], |row| {
        let dl: String = row.get(0)?;
        let label: String = row.get(1)?;
        let fs: String = row.get(2)?;
        Ok((dl, label, fs))
    }) {
        Ok(rows) => rows,
        Err(_) => return volumes,
    };

    for row in vol_rows.flatten() {
        let Some(letter) = row.0.chars().next() else {
            continue;
        };
        volumes.insert(
            letter,
            IndexedVolume {
                label: row.1,
                file_system: row.2,
                last_scan: Instant::now(),
                items: Vec::new(),
                live_paths: HashSet::new(),
            },
        );
    }

    if volumes.is_empty() {
        return volumes;
    }

    let mut item_stmt =
        match conn.prepare("SELECT drive_letter, name, full_path, is_dir FROM session_items") {
            Ok(s) => s,
            Err(_) => return volumes,
        };

    let item_rows = match item_stmt.query_map([], |row| {
        let dl: String = row.get(0)?;
        let name: String = row.get(1)?;
        let full_path: String = row.get(2)?;
        let is_dir: bool = row.get(3)?;
        Ok((dl, name, full_path, is_dir))
    }) {
        Ok(rows) => rows,
        Err(_) => return volumes,
    };

    for (dl, name, full_path, is_dir) in item_rows.flatten() {
        let Some(letter) = dl.chars().next() else {
            continue;
        };
        let Some(volume) = volumes.get_mut(&letter) else {
            continue;
        };

        let path_key = normalize_path_key(Path::new(&full_path));
        volume.items.push(IndexedItem {
            name_lower: name.to_lowercase(),
            name,
            full_path,
            path_key: path_key.clone(),
            is_dir,
        });
        volume.live_paths.insert(path_key);
    }

    volumes.retain(|_, v| !v.items.is_empty());

    volumes
}

pub(super) fn save_volume(conn: &rusqlite::Connection, drive_letter: char, items: &[IndexedItem]) {
    let dl = drive_letter.to_string();

    let tx = match conn.execute("BEGIN IMMEDIATE", []) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("[SESSION-SEARCH] Failed to begin transaction: {}", e);
            false
        }
    };

    let _ = conn.execute("DELETE FROM session_items WHERE drive_letter = ?1", [&dl]);

    if let Ok(mut stmt) = conn.prepare(
        "INSERT INTO session_items (drive_letter, name, full_path, is_dir) VALUES (?1, ?2, ?3, ?4)",
    ) {
        for item in items {
            let _ = stmt.execute(rusqlite::params![
                dl,
                item.name,
                item.full_path,
                item.is_dir
            ]);
        }
    }

    let _ = conn.execute(
        "INSERT OR REPLACE INTO session_volumes (drive_letter, label, file_system) VALUES (?1, '', '')",
        [&dl],
    );

    if tx {
        let _ = conn.execute("COMMIT", []);
    }

    log::debug!(
        "[SESSION-SEARCH] Persisted {} items for {}:\\",
        items.len(),
        drive_letter
    );
}

pub(super) fn delete_volume(conn: &rusqlite::Connection, drive_letter: char) {
    let dl = drive_letter.to_string();
    let _ = conn.execute("DELETE FROM session_items WHERE drive_letter = ?1", [&dl]);
    let _ = conn.execute("DELETE FROM session_volumes WHERE drive_letter = ?1", [&dl]);
}
