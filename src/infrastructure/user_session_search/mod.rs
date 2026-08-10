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
mod trigram;
mod watcher_retry;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mtt_search_protocol::SearchResultItem;

use crate::infrastructure::drive_watcher::{DriveWatcher, DriveWatcherEvent};
use watcher_retry::WatcherRetryState;

const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(12);
const MAX_ITEMS_PER_VOLUME: usize = 1_500_000;
/// PERF-06: global cap on concurrent background volume rescans. Excess
/// volumes wait for the next discovery cycle instead of spawning more
/// full-volume walkers at once.
const MAX_CONCURRENT_SESSION_RESCANS: usize = 2;

#[derive(Clone)]
struct IndexedItem {
    /// PERF-06: `Arc<str>` fields halve the per-item stack footprint versus
    /// `String` and let `path_key` be shared with `live_paths` (previously a
    /// full heap copy of every path).
    name: Arc<str>,
    name_lower: Arc<str>,
    full_path: Arc<str>,
    path_key: Arc<str>,
    is_dir: bool,
    size: u64,
}

struct IndexedVolume {
    label: String,
    file_system: String,
    last_scan: Instant,
    items: Vec<IndexedItem>,
    live_paths: HashSet<Arc<str>>,
    needs_rescan: bool,
    /// PERF-06: trigram index over `items` for sub-linear per-keystroke
    /// search. `None` for volumes above `trigram::TRIGRAM_INDEX_MAX_ITEMS`
    /// (those keep the pre-PERF-06 linear scan).
    trigram_index: Option<trigram::TrigramIndex>,
}

struct CandidateVolume {
    drive_letter: char,
    label: String,
    file_system: String,
}

/// PERF-06: result delivered by a background volume-rescan thread.
struct RescanDelivery {
    drive_letter: char,
    label: String,
    file_system: String,
    outcome: Result<scanner::ScanOutcome, String>,
}

/// In-process search index used for user-session-only mounts.
pub struct UserSessionSearchIndex {
    volumes: HashMap<char, IndexedVolume>,
    watchers: HashMap<char, DriveWatcher>,
    watcher_retries: HashMap<char, WatcherRetryState>,
    active_letters: HashSet<char>,
    last_discovery: Option<Instant>,
    /// Optional SQLite connection for persisting/loading indexed items.
    db: Option<rusqlite::Connection>,
    /// PERF-06: deliveries from background rescan threads.
    rescan_rx: std::sync::mpsc::Receiver<RescanDelivery>,
    rescan_tx: std::sync::mpsc::Sender<RescanDelivery>,
    /// Volumes with an in-flight background scan (prevents duplicate scans).
    rescanning: HashSet<char>,
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

        let (rescan_tx, rescan_rx) = std::sync::mpsc::channel::<RescanDelivery>();

        Self {
            volumes,
            watchers: HashMap::new(),
            watcher_retries: HashMap::new(),
            active_letters: HashSet::new(),
            last_discovery: None,
            db,
            rescan_rx,
            rescan_tx,
            rescanning: HashSet::new(),
        }
    }

    /// Apply pending filesystem events only (no discovery/full scan).
    pub fn poll_fast_updates(&mut self) {
        // PERF-06: apply any completed background rescans first so searches
        // see fresh data without blocking on a scan.
        self.apply_completed_rescans();
        self.apply_pending_events();
        let active_letters = self.active_letters.clone();
        self.sync_watchers(&active_letters);
    }

    /// PERF-06: drain completed background rescan deliveries.
    ///
    /// Exposed so the search worker can drain while idle (overlay closed);
    /// otherwise finished scans — each holding a full volume's worth of
    /// items — would linger in the channel until the next request.
    pub fn drain_completed_rescans(&mut self) {
        self.apply_completed_rescans();
    }

    /// PERF-06: drain completed background rescan deliveries.
    fn apply_completed_rescans(&mut self) {
        while let Ok(delivery) = self.rescan_rx.try_recv() {
            self.rescanning.remove(&delivery.drive_letter);

            // Volume is no longer active (unmounted/removed while scanning):
            // discard the scan instead of resurrecting it.
            if !self.active_letters.contains(&delivery.drive_letter) {
                log::debug!(
                    "[SESSION-SEARCH] Discarding rescan result for inactive volume {}:",
                    delivery.drive_letter
                );
                continue;
            }

            match delivery.outcome {
                Ok(scan) => {
                    let count = scan.items.len();

                    if let Some(conn) = &self.db {
                        db::save_volume(conn, delivery.drive_letter, &scan.items);
                    }

                    self.volumes.insert(
                        delivery.drive_letter,
                        IndexedVolume {
                            label: delivery.label,
                            file_system: delivery.file_system,
                            last_scan: Instant::now(),
                            items: scan.items,
                            live_paths: scan.live_paths,
                            needs_rescan: false,
                            trigram_index: scan.trigram_index,
                        },
                    );
                    log::debug!(
                        "[SESSION-SEARCH] {}:\\ indexed {} entries in {:.2}s (dirs: {}, errors: {}) [background]",
                        delivery.drive_letter,
                        count,
                        scan.elapsed.as_secs_f64(),
                        scan.directories_scanned,
                        scan.errors
                    );
                }
                Err(error) => {
                    log::warn!(
                        "[SESSION-SEARCH] {}:\\ background scan failed: {}",
                        delivery.drive_letter,
                        error
                    );
                }
            }
        }
    }

    /// PERF-06: schedule a full-volume scan on a dedicated detached thread so
    /// a slow FUSE/virtual mount cannot block the search worker (which would
    /// also stall service-backed searches). At most one scan per volume and at
    /// most `MAX_CONCURRENT_SESSION_RESCANS` scans globally are in flight; a
    /// skipped volume is retried on the next discovery cycle.
    fn schedule_rescan(&mut self, candidate: &CandidateVolume) {
        if self.rescanning.contains(&candidate.drive_letter) {
            return;
        }
        if self.rescanning.len() >= MAX_CONCURRENT_SESSION_RESCANS {
            log::debug!(
                "[SESSION-SEARCH] Rescan cap reached; deferring {}:\\ to next cycle",
                candidate.drive_letter
            );
            return;
        }
        self.rescanning.insert(candidate.drive_letter);

        let drive_letter = candidate.drive_letter;
        let label = candidate.label.clone();
        let file_system = candidate.file_system.clone();
        let tx = self.rescan_tx.clone();

        let spawn_result = std::thread::Builder::new()
            .name(format!("session-rescan-{drive_letter}"))
            .spawn(move || {
                let outcome = scanner::scan_volume(drive_letter).map_err(|e| e.to_string());
                let _ = tx.send(RescanDelivery {
                    drive_letter,
                    label,
                    file_system,
                    outcome,
                });
            });

        if let Err(error) = spawn_result {
            self.rescanning.remove(&candidate.drive_letter);
            log::warn!(
                "[SESSION-SEARCH] Failed to spawn background rescan for {}:\\: {}",
                candidate.drive_letter,
                error
            );
        }
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
                    self.poll_fast_updates();
                    return;
                }
            }
        }

        self.last_discovery = Some(Instant::now());
        // PERF-06: pick up completed background rescans before evaluating
        // staleness, so freshly scanned volumes are not rescheduled.
        self.apply_completed_rescans();
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
                        || existing.needs_rescan
                        || existing.file_system != candidate.file_system
                        || existing.label != candidate.label
                })
                .unwrap_or(true);

            if should_rescan {
                stale_candidates.push(candidate);
            }
        }

        self.apply_pending_events();
        self.active_letters = active_letters.clone();
        self.sync_watchers(&active_letters);

        // PERF-06: full volume scans run on dedicated background threads
        // (was: inline on the search worker, blocking all searches).
        for candidate in &stale_candidates {
            self.schedule_rescan(candidate);
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
        self.watcher_retries
            .retain(|letter, _| active_letters.contains(letter));
        // PERF-06: drop in-flight scan markers for volumes that disappeared;
        // late deliveries are discarded in apply_completed_rescans.
        self.rescanning
            .retain(|letter| active_letters.contains(letter));
        self.active_letters = active_letters;
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchResultItem> {
        self.search_page(query, 0, limit).0
    }

    pub fn count_matches(&self, query: &str) -> u32 {
        if query.is_empty() {
            return 0;
        }

        let query_lower = query.to_lowercase();
        let tokens: Vec<&str> = query_lower.split_whitespace().collect();
        let mut matched = 0usize;

        for volume in self.volumes.values() {
            visit_volume_matches(volume, &tokens, |_item| {
                matched = matched.saturating_add(1);
                true
            });
        }

        matched.min(u32::MAX as usize) as u32
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
            let mut page_full = false;
            visit_volume_matches(volume, &tokens, |item| {
                if matched < offset {
                    matched += 1;
                    return true;
                }

                if results.len() >= limit {
                    page_full = true;
                    return false;
                }

                results.push(SearchResultItem {
                    name: item.name.to_string(),
                    full_path: item.full_path.to_string(),
                    is_dir: item.is_dir,
                    size: item.size,
                });
                matched += 1;
                true
            });
            if page_full {
                return (results, true);
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
        let now = Instant::now();
        self.watchers
            .retain(|letter, _| active_letters.contains(letter));
        self.watcher_retries
            .retain(|letter, _| active_letters.contains(letter));

        let dead_letters: Vec<char> = self
            .watchers
            .iter()
            .filter_map(|(letter, watcher)| watcher.is_stopped().then_some(*letter))
            .collect();

        for letter in dead_letters {
            if let Some(watcher) = self.watchers.remove(&letter) {
                if let Some(volume) = self.volumes.get_mut(&letter) {
                    for event in watcher.poll_events() {
                        if matches!(event, DriveWatcherEvent::DriveLost(_)) {
                            self.last_discovery = None;
                        }
                        scanner::apply_event_to_volume(volume, &event);
                    }
                }
            }
            let retry = WatcherRetryState::after_failure(self.watcher_retries.get(&letter), now);
            log::warn!(
                "[SESSION-SEARCH] {}:\\ watcher stopped; retry {} in {:.1}s",
                letter,
                retry.failures(),
                retry.retry_in(now).as_secs_f32()
            );
            self.watcher_retries.insert(letter, retry);
        }

        for letter in active_letters {
            if let Some(watcher) = self.watchers.get(letter) {
                if watcher.is_running() {
                    self.watcher_retries.remove(letter);
                }
                continue;
            }

            if self
                .watcher_retries
                .get(letter)
                .is_some_and(|retry| !retry.is_ready(now))
            {
                continue;
            }

            let root = PathBuf::from(format!("{}:\\", letter));
            if let Some(watcher) = DriveWatcher::new(root.clone(), root) {
                self.watchers.insert(*letter, watcher);
            } else {
                let retry = WatcherRetryState::after_failure(self.watcher_retries.get(letter), now);
                self.watcher_retries.insert(*letter, retry);
            }
        }
    }

    fn apply_pending_events(&mut self) {
        let mut drive_lost = false;
        for (letter, watcher) in &self.watchers {
            let Some(volume) = self.volumes.get_mut(letter) else {
                continue;
            };

            for event in watcher.poll_events() {
                drive_lost |= matches!(event, DriveWatcherEvent::DriveLost(_));
                scanner::apply_event_to_volume(volume, &event);
            }
        }
        if drive_lost {
            self.last_discovery = None;
        }
    }
}

fn item_matches_query(volume: &IndexedVolume, item: &IndexedItem, tokens: &[&str]) -> bool {
    volume.live_paths.contains(&item.path_key)
        && tokens.iter().all(|token| item.name_lower.contains(token))
}

/// PERF-06: visits every matching item of a volume in items-vector order
/// (identical order to the pre-PERF-06 linear scan), calling `visit` for
/// each one. `visit` returns `false` to stop early.
///
/// When the volume has a trigram index, candidate item indices are narrowed
/// per token and then filtered by the exact predicate — the match set is
/// identical to the linear scan because trigram containment is necessary for
/// `name_lower.contains(token)`. Without an index (or when all tokens are
/// shorter than 3 chars) this degrades to the plain linear scan.
fn visit_volume_matches<F>(volume: &IndexedVolume, tokens: &[&str], mut visit: F)
where
    F: FnMut(&IndexedItem) -> bool,
{
    let candidate_indices: Option<Vec<u32>> =
        volume.trigram_index.as_ref().and_then(|index| {
            let mut acc: Option<Vec<u32>> = None;
            for token in tokens {
                // `None` = token too short to narrow; the exact predicate
                // still applies to it during verification.
                let Some(token_candidates) = index.candidates_for_token(token) else {
                    continue;
                };
                acc = Some(match acc {
                    None => token_candidates,
                    Some(prev) => intersect_sorted(prev, token_candidates),
                });
                if acc.as_ref().is_some_and(|candidates| candidates.is_empty()) {
                    break;
                }
            }
            acc
        });

    match candidate_indices {
        Some(indices) => {
            for idx in indices {
                let Some(item) = volume.items.get(idx as usize) else {
                    continue;
                };
                if item_matches_query(volume, item, tokens) && !visit(item) {
                    return;
                }
            }
        }
        None => {
            for item in &volume.items {
                if item_matches_query(volume, item, tokens) && !visit(item) {
                    return;
                }
            }
        }
    }
}

/// Intersection of two sorted ascending `u32` vectors.
fn intersect_sorted(mut a: Vec<u32>, b: Vec<u32>) -> Vec<u32> {
    a.retain(|idx| b.binary_search(idx).is_ok());
    a
}

impl Default for UserSessionSearchIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use super::{trigram, visit_volume_matches, IndexedItem, IndexedVolume};

    /// Builds a volume from plain names. `with_index` controls whether the
    /// trigram index is attached, so tests can compare both paths.
    fn test_volume(names: &[&str], with_index: bool) -> IndexedVolume {
        let mut items = Vec::new();
        let mut live_paths = std::collections::HashSet::new();
        for name in names {
            let path_key: Arc<str> = Arc::from(format!("c:\\{}", name).to_ascii_lowercase());
            live_paths.insert(path_key.clone());
            items.push(IndexedItem {
                name_lower: Arc::from(name.to_lowercase()),
                name: Arc::from(*name),
                full_path: Arc::from(format!("c:\\{}", name)),
                path_key,
                is_dir: false,
                size: 1,
            });
        }
        let trigram_index = if with_index {
            Some(trigram::TrigramIndex::build(
                items.iter().map(|item| &*item.name_lower),
            ))
        } else {
            None
        };
        IndexedVolume {
            label: String::new(),
            file_system: String::new(),
            last_scan: Instant::now(),
            items,
            live_paths,
            needs_rescan: false,
            trigram_index,
        }
    }

    /// Brute-force reference: matches by the exact original predicate.
    fn linear_matches(volume: &IndexedVolume, tokens: &[&str]) -> Vec<usize> {
        volume
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                volume.live_paths.contains(&item.path_key)
                    && tokens.iter().all(|token| item.name_lower.contains(token))
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    fn indexed_visit_matches(volume: &IndexedVolume, tokens: &[&str]) -> Vec<usize> {
        let mut matches = Vec::new();
        visit_volume_matches(volume, tokens, |item| {
            let idx = volume
                .items
                .iter()
                .position(|candidate| Arc::ptr_eq(&candidate.path_key, &item.path_key))
                .unwrap();
            matches.push(idx);
            true
        });
        matches
    }

    const PARITY_NAMES: &[&str] = &[
        "photo 2024.jpg",
        "Photo-2024-copy.jpg",
        "annual-report.pdf",
        "report.txt",
        "résumé.txt",
        "RÉSUMÉ_FINAL.docx",
        "ab",
        "a",
        "aaaa.txt",
        "zebra-stripes.png",
        "2024 vacation.mp4",
        "PHOTO_2024_RAW.CR2",
    ];

    const PARITY_QUERIES: &[&[&str]] = &[
        &["photo"],
        &["photo", "2024"],
        &["2024", "photo"],
        &["report"],
        &["résumé"],
        &["résumé", "final"],
        &["ab"],
        &["a"],
        &["aaaa"],
        &["zebra"],
        &["nonexistent-token"],
        &["photo", "zz"],
        &["an", "report"],
        &["2024"],
        &["photo_2024_raw"],
    ];

    #[test]
    fn trigram_path_matches_linear_scan_exactly() {
        let with_index = test_volume(PARITY_NAMES, true);
        let without_index = test_volume(PARITY_NAMES, false);

        for tokens in PARITY_QUERIES {
            let expected = linear_matches(&without_index, tokens);
            let via_linear = indexed_visit_matches(&without_index, tokens);
            let via_index = indexed_visit_matches(&with_index, tokens);
            assert_eq!(
                via_linear, expected,
                "linear visitor diverged for tokens {:?}",
                tokens
            );
            assert_eq!(
                via_index, expected,
                "trigram path diverged for tokens {:?}",
                tokens
            );
        }
    }

    #[test]
    fn dead_paths_are_excluded_on_both_paths() {
        let mut volume = test_volume(PARITY_NAMES, true);
        // Mark one matching item as deleted.
        let dead_key: Arc<str> = Arc::from("c:\\report.txt");
        volume.live_paths.remove(&dead_key);

        let expected = linear_matches(&volume, &["report"]);
        let via_index = indexed_visit_matches(&volume, &["report"]);
        assert_eq!(via_index, expected);
        assert!(!expected.contains(&3)); // "report.txt" is dead
        assert!(expected.contains(&2)); // "annual-report.pdf" still live
    }

    #[test]
    fn incremental_upsert_keeps_parity() {
        let mut volume = test_volume(PARITY_NAMES, true);

        // Simulate a watcher upsert appending a new item (index updated
        // incrementally, like scanner::upsert_path does).
        let new_name = "new-photo-2025.jpg";
        let path_key: Arc<str> = Arc::from(format!("c:\\{}", new_name));
        volume.live_paths.insert(path_key.clone());
        volume.items.push(IndexedItem {
            name_lower: Arc::from(new_name.to_lowercase()),
            name: Arc::from(new_name),
            full_path: Arc::from(format!("c:\\{}", new_name)),
            path_key,
            is_dir: false,
            size: 2,
        });
        volume
            .trigram_index
            .as_mut()
            .unwrap()
            .insert_name(volume.items.len() - 1, &new_name.to_lowercase());

        let expected = linear_matches(&volume, &["photo", "2025"]);
        let via_index = indexed_visit_matches(&volume, &["photo", "2025"]);
        assert_eq!(via_index, expected);
        assert_eq!(expected, vec![volume.items.len() - 1]);
    }
}
