//! User-session search index for volumes that the Windows service cannot see.
//!
//! Main use case: virtual mounts exposed only in the interactive user session
//! (e.g. Cryptomator/CryptoFS via WinFsp/FUSE).
//!
//! Persists indexed items to a local SQLite database so that results are
//! available immediately on the next app startup (before the first rescan
//! completes).

mod db;
mod discovery;
mod scanner;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use mtt_search_protocol::SearchResultItem;

use crate::infrastructure::drive_watcher::DriveWatcher;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(12);
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
        let db = db::open_session_db();
        let volumes = match &db {
            Some(conn) => db::load_all_volumes(conn),
            None => HashMap::new(),
        };

        if !volumes.is_empty() {
            let total: usize = volumes.values().map(|v| v.items.len()).sum();
            let drives: Vec<char> = volumes.keys().copied().collect();
            log::debug!(
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
        let mut candidates = discovery::discover_candidate_volumes(service_volumes, service_online);
        candidates.sort_by_key(|c| c.drive_letter);

        let mut active_letters = HashSet::with_capacity(candidates.len());
        let mut stale_candidates = Vec::new();

        for candidate in candidates {
            active_letters.insert(candidate.drive_letter);

            let rescan_interval =
                discovery::rescan_interval_for_volume(&candidate.file_system, &candidate.label);

            let should_rescan = self
                .volumes
                .get(&candidate.drive_letter)
                .map(|existing| {
                    existing.last_scan.elapsed() >= rescan_interval
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

        for candidate in &stale_candidates {
            match scanner::scan_volume(candidate.drive_letter) {
                Ok(scan) => {
                    let count = scan.items.len();

                    if let Some(conn) = &self.db {
                        db::save_volume(conn, candidate.drive_letter, &scan.items);
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
                    log::debug!(
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

        let removed_letters: Vec<char> = self
            .volumes
            .keys()
            .filter(|letter| !active_letters.contains(letter))
            .copied()
            .collect();
        for letter in &removed_letters {
            if let Some(conn) = &self.db {
                db::delete_volume(conn, *letter);
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
        let tokens: Vec<&str> = query_lower.split_whitespace().collect();
        let mut results = Vec::with_capacity(limit.min(128));
        let mut matched = 0usize;

        for volume in self.volumes.values() {
            for item in &volume.items {
                if !volume.live_paths.contains(&item.path_key) {
                    continue;
                }

                if !tokens.iter().all(|token| item.name_lower.contains(token)) {
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
                scanner::apply_event_to_volume(volume, &event);
            }
        }
    }
}

impl Default for UserSessionSearchIndex {
    fn default() -> Self {
        Self::new()
    }
}
