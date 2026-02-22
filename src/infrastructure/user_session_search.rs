//! User-session search index for volumes that the Windows service cannot see.
//!
//! Main use case: virtual mounts exposed only in the interactive user session
//! (e.g. Cryptomator/CryptoFS via WinFsp/FUSE).
//!
//! Persists indexed items to a local SQLite database so that results are
//! available immediately on the next app startup (before the first rescan
//! completes).

use std::collections::{HashMap, HashSet, VecDeque};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mtt_search_protocol::SearchResultItem;

use crate::infrastructure::drive_watcher::{DriveWatcher, DriveWatcherEvent};

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(12);
const RESCAN_INTERVAL: Duration = Duration::from_secs(300);
const MAX_ITEMS_PER_VOLUME: usize = 1_500_000;

#[derive(Clone)]
struct IndexedItem {
    name: String,
    name_lower: String,
    full_path: String,
    path_key: String,
    is_dir: bool,
}

struct IndexedVolume {
    label: String,
    file_system: String,
    last_scan: Instant,
    items: Vec<IndexedItem>,
    live_paths: HashSet<String>,
}

struct CandidateVolume {
    drive_letter: char,
    label: String,
    file_system: String,
}

struct ScanOutcome {
    items: Vec<IndexedItem>,
    live_paths: HashSet<String>,
    directories_scanned: usize,
    errors: usize,
    elapsed: Duration,
}

/// In-process search index used for user-session-only mounts.
pub struct UserSessionSearchIndex {
    volumes: HashMap<char, IndexedVolume>,
    watchers: HashMap<char, DriveWatcher>,
    last_discovery: Option<Instant>,
    /// Optional SQLite connection for persisting/loading indexed items.
    db: Option<rusqlite::Connection>,
}

impl UserSessionSearchIndex {
    pub fn new() -> Self {
        let db = open_session_db();
        let volumes = match &db {
            Some(conn) => load_all_volumes(conn),
            None => HashMap::new(),
        };

        if !volumes.is_empty() {
            let total: usize = volumes.values().map(|v| v.items.len()).sum();
            let drives: Vec<char> = volumes.keys().copied().collect();
            log::info!(
                "[SESSION-SEARCH] Loaded {} cached entries from {} volume(s) {:?}",
                total,
                drives.len(),
                drives
            );
        }

        Self {
            volumes,
            watchers: HashMap::new(),
            last_discovery: None,
            db,
        }
    }

    /// Apply pending filesystem events only (no discovery/full scan).
    pub fn poll_fast_updates(&mut self) {
        self.apply_pending_events();
    }

    /// Refresh candidate volume set and rescan stale/new volumes.
    ///
    /// `service_online` indicates whether service status can be trusted now.
    /// - If online: index drives missing from service coverage.
    /// - If offline: index only strongly virtual drives (to avoid scanning C:\).
    pub fn refresh(
        &mut self,
        service_volumes: &HashSet<char>,
        service_online: bool,
        force_discovery: bool,
    ) {
        if !force_discovery {
            if let Some(last) = self.last_discovery {
                if last.elapsed() < DISCOVERY_INTERVAL {
                    self.apply_pending_events();
                    return;
                }
            }
        }

        self.last_discovery = Some(Instant::now());
        let mut candidates = discover_candidate_volumes(service_volumes, service_online);
        candidates.sort_by_key(|c| c.drive_letter);

        let mut active_letters = HashSet::with_capacity(candidates.len());
        let mut stale_candidates = Vec::new();

        for candidate in candidates {
            active_letters.insert(candidate.drive_letter);

            let should_rescan = self
                .volumes
                .get(&candidate.drive_letter)
                .map(|existing| {
                    existing.last_scan.elapsed() >= RESCAN_INTERVAL
                        || existing.file_system != candidate.file_system
                        || existing.label != candidate.label
                })
                .unwrap_or(true);

            if should_rescan {
                stale_candidates.push(candidate);
            }
        }

        self.sync_watchers(&active_letters);
        self.apply_pending_events();

        // Scan ALL stale candidates at once (not just one per cycle) so that
        // results for every non-NTFS/virtual volume become available quickly.
        for candidate in &stale_candidates {
            match scan_volume(candidate.drive_letter) {
                Ok(scan) => {
                    let count = scan.items.len();

                    // Persist to SQLite so next startup is instant.
                    if let Some(conn) = &self.db {
                        save_volume(conn, candidate.drive_letter, &scan.items);
                    }

                    self.volumes.insert(
                        candidate.drive_letter,
                        IndexedVolume {
                            label: candidate.label.clone(),
                            file_system: candidate.file_system.clone(),
                            last_scan: Instant::now(),
                            items: scan.items,
                            live_paths: scan.live_paths,
                        },
                    );
                    log::info!(
                        "[SESSION-SEARCH] {}:\\ indexed {} entries in {:.2}s (dirs: {}, errors: {})",
                        candidate.drive_letter,
                        count,
                        scan.elapsed.as_secs_f64(),
                        scan.directories_scanned,
                        scan.errors
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[SESSION-SEARCH] {}:\\ scan failed: {}",
                        candidate.drive_letter,
                        e
                    );
                }
            }
        }

        // Remove volumes that are no longer active candidates.
        let removed_letters: Vec<char> = self
            .volumes
            .keys()
            .filter(|letter| !active_letters.contains(letter))
            .copied()
            .collect();
        for letter in &removed_letters {
            if let Some(conn) = &self.db {
                delete_volume(conn, *letter);
            }
        }

        self.volumes
            .retain(|letter, _| active_letters.contains(letter));
        self.watchers
            .retain(|letter, _| active_letters.contains(letter));
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.search_page(query, 0, limit).0
    }

    pub fn search_page(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> (Vec<SearchResultItem>, bool) {
        if query.is_empty() || limit == 0 {
            return (Vec::new(), false);
        }

        let query_lower = query.to_lowercase();
        let mut results = Vec::with_capacity(limit.min(128));
        let mut matched = 0usize;

        for volume in self.volumes.values() {
            for item in &volume.items {
                if !volume.live_paths.contains(&item.path_key) {
                    continue;
                }

                if !item.name_lower.contains(&query_lower) {
                    continue;
                }

                if matched < offset {
                    matched += 1;
                    continue;
                }

                if results.len() >= limit {
                    return (results, true);
                }

                results.push(SearchResultItem {
                    name: item.name.clone(),
                    full_path: item.full_path.clone(),
                    is_dir: item.is_dir,
                    size: 0,
                });
                matched += 1;
            }
        }

        (results, false)
    }

    pub fn total_indexed(&self) -> u64 {
        self.volumes
            .values()
            .map(|v| v.live_paths.len() as u64)
            .sum::<u64>()
    }

    pub fn has_indexed_items(&self) -> bool {
        self.volumes.values().any(|v| !v.live_paths.is_empty())
    }

    fn sync_watchers(&mut self, active_letters: &HashSet<char>) {
        for letter in active_letters {
            if self.watchers.contains_key(letter) {
                continue;
            }

            let root = PathBuf::from(format!("{}:\\", letter));
            if let Some(watcher) = DriveWatcher::new(root.clone(), root) {
                self.watchers.insert(*letter, watcher);
            }
        }
    }

    fn apply_pending_events(&mut self) {
        for (letter, watcher) in &self.watchers {
            let Some(volume) = self.volumes.get_mut(letter) else {
                continue;
            };

            for event in watcher.poll_events() {
                apply_event_to_volume(volume, &event);
            }
        }
    }
}

impl Default for UserSessionSearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn apply_event_to_volume(volume: &mut IndexedVolume, event: &DriveWatcherEvent) {
    match event {
        DriveWatcherEvent::Created(path) | DriveWatcherEvent::Modified(path) => {
            upsert_path(volume, path);
        }
        DriveWatcherEvent::Deleted(path) => {
            volume.live_paths.remove(&normalize_path_key(path));
        }
        DriveWatcherEvent::Renamed(old_path, new_path) => {
            volume.live_paths.remove(&normalize_path_key(old_path));
            upsert_path(volume, new_path);
        }
        DriveWatcherEvent::Unknown(_) => {}
        DriveWatcherEvent::DriveLost(_) => {
            volume.live_paths.clear();
        }
    }
}

fn upsert_path(volume: &mut IndexedVolume, path: &Path) {
    if !crate::infrastructure::onedrive::fast_path_exists(path) {
        return;
    }

    let Some(name_os) = path.file_name() else {
        return;
    };
    let name = name_os.to_string_lossy().into_owned();
    if name.is_empty() {
        return;
    }

    let key = normalize_path_key(path);
    if volume.live_paths.contains(&key) {
        return;
    }

    let full_path = path.to_string_lossy().into_owned();
    volume.items.push(IndexedItem {
        name_lower: name.to_lowercase(),
        name,
        full_path,
        path_key: key.clone(),
        is_dir: crate::infrastructure::onedrive::fast_is_dir(path),
    });
    volume.live_paths.insert(key);
}

fn normalize_path_key(path: &Path) -> String {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let stripped = lower.strip_prefix(r"\\?\").unwrap_or(&lower);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_string()
    } else {
        stripped.to_string()
    }
}

fn discover_candidate_volumes(
    service_volumes: &HashSet<char>,
    service_online: bool,
) -> Vec<CandidateVolume> {
    let mut candidates = Vec::new();
    let drives = crate::infrastructure::windows::get_all_drives();

    for (path, label) in drives {
        let Some(letter) = parse_drive_letter(&path) else {
            continue;
        };

        let volume = crate::infrastructure::windows::get_volume_info(&path);
        let file_system = volume.file_system;

        if should_index_volume(
            letter,
            &label,
            &file_system,
            service_volumes,
            service_online,
        ) {
            candidates.push(CandidateVolume {
                drive_letter: letter,
                label,
                file_system,
            });
        }
    }

    candidates
}

fn should_index_volume(
    drive_letter: char,
    label: &str,
    file_system: &str,
    service_volumes: &HashSet<char>,
    service_online: bool,
) -> bool {
    let missing_from_service = !service_volumes.contains(&drive_letter);
    if !missing_from_service {
        return false;
    }

    let virtual_indicator = is_virtual_indicator(label, file_system);
    if service_online {
        return virtual_indicator || !is_usn_filesystem(file_system);
    }

    virtual_indicator
}

fn is_virtual_indicator(label: &str, file_system: &str) -> bool {
    let label_lower = label.to_ascii_lowercase();
    let fs_lower = file_system.to_ascii_lowercase();

    label_lower.contains("cryptomator")
        || fs_lower.contains("cryptofs")
        || fs_lower.contains("dokan")
        || fs_lower.contains("winfsp")
        || fs_lower == "fuse"
}

fn is_usn_filesystem(file_system: &str) -> bool {
    file_system.eq_ignore_ascii_case("NTFS") || file_system.eq_ignore_ascii_case("ReFS")
}

fn parse_drive_letter(path: &str) -> Option<char> {
    path.chars()
        .next()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c.is_ascii_alphabetic())
}

fn scan_volume(drive_letter: char) -> Result<ScanOutcome, String> {
    let root = PathBuf::from(format!("{}:\\", drive_letter));
    if !root.exists() {
        return Err(format!("{}:\\ root is not accessible", drive_letter));
    }

    let start = Instant::now();
    let mut queue = VecDeque::new();
    let mut items = Vec::new();
    let mut live_paths = HashSet::new();
    let mut directories_scanned = 0usize;
    let mut errors = 0usize;

    queue.push_back(root);

    'scan: while let Some(dir_path) = queue.pop_front() {
        directories_scanned += 1;

        let entries = match std::fs::read_dir(&dir_path) {
            Ok(entries) => entries,
            Err(_) => {
                errors += 1;
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };

            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.is_empty() {
                continue;
            }

            let path_key = normalize_path_key(&path);
            let is_dir = file_type.is_dir();
            items.push(IndexedItem {
                name_lower: name.to_lowercase(),
                name,
                full_path: path.to_string_lossy().into_owned(),
                path_key: path_key.clone(),
                is_dir,
            });
            live_paths.insert(path_key);

            if items.len() >= MAX_ITEMS_PER_VOLUME {
                break 'scan;
            }

            if !is_dir || file_type.is_symlink() {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    errors += 1;
                    continue;
                }
            };

            if (metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                continue;
            }

            queue.push_back(path);
        }
    }

    Ok(ScanOutcome {
        items,
        live_paths,
        directories_scanned,
        errors,
        elapsed: start.elapsed(),
    })
}

// ── SQLite persistence ──────────────────────────────────────────────────

/// Open (or create) the session-search database.
fn open_session_db() -> Option<rusqlite::Connection> {
    let cache_dir = dirs::data_local_dir()?.join("MTT-File-Manager").join("thumbnails");
    let db_path = cache_dir.join("session_search.db");

    if let Err(e) = std::fs::create_dir_all(&cache_dir) {
        log::warn!("[SESSION-SEARCH] Failed to create cache dir {:?}: {}", cache_dir, e);
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

/// Load all persisted volumes from SQLite into in-memory structures.
fn load_all_volumes(conn: &rusqlite::Connection) -> HashMap<char, IndexedVolume> {
    let mut volumes = HashMap::new();

    // Load volume metadata.
    let mut vol_stmt = match conn.prepare(
        "SELECT drive_letter, label, file_system FROM session_volumes",
    ) {
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
        let Some(letter) = row.0.chars().next() else { continue };
        volumes.insert(
            letter,
            IndexedVolume {
                label: row.1,
                file_system: row.2,
                last_scan: Instant::now(), // treat cached data as fresh for initial serving
                items: Vec::new(),
                live_paths: HashSet::new(),
            },
        );
    }

    if volumes.is_empty() {
        return volumes;
    }

    // Load items per volume.
    let mut item_stmt = match conn.prepare(
        "SELECT drive_letter, name, full_path, is_dir FROM session_items",
    ) {
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
        let Some(letter) = dl.chars().next() else { continue };
        let Some(volume) = volumes.get_mut(&letter) else { continue };

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

    // Remove empty volumes (items may have been pruned).
    volumes.retain(|_, v| !v.items.is_empty());

    volumes
}

/// Replace all persisted items for a volume with the fresh scan.
fn save_volume(conn: &rusqlite::Connection, drive_letter: char, items: &[IndexedItem]) {
    let dl = drive_letter.to_string();

    let tx = match conn.execute("BEGIN IMMEDIATE", []) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("[SESSION-SEARCH] Failed to begin transaction: {}", e);
            false
        }
    };

    // Delete old items for this drive.
    let _ = conn.execute("DELETE FROM session_items WHERE drive_letter = ?1", [&dl]);

    // Insert new items in batches.
    if let Ok(mut stmt) = conn.prepare(
        "INSERT INTO session_items (drive_letter, name, full_path, is_dir) VALUES (?1, ?2, ?3, ?4)",
    ) {
        for item in items {
            let _ = stmt.execute(rusqlite::params![dl, item.name, item.full_path, item.is_dir]);
        }
    }

    // Upsert volume metadata.
    let _ = conn.execute(
        "INSERT OR REPLACE INTO session_volumes (drive_letter, label, file_system) VALUES (?1, '', '')",
        [&dl],
    );

    if tx {
        let _ = conn.execute("COMMIT", []);
    }

    log::info!(
        "[SESSION-SEARCH] Persisted {} items for {}:\\",
        items.len(),
        drive_letter
    );
}

/// Remove persisted data for a volume that is no longer active.
fn delete_volume(conn: &rusqlite::Connection, drive_letter: char) {
    let dl = drive_letter.to_string();
    let _ = conn.execute("DELETE FROM session_items WHERE drive_letter = ?1", [&dl]);
    let _ = conn.execute("DELETE FROM session_volumes WHERE drive_letter = ?1", [&dl]);
}
