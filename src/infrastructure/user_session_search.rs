//! User-session search index for volumes that the Windows service cannot see.
//!
//! Main use case: virtual mounts exposed only in the interactive user session
//! (e.g. Cryptomator/CryptoFS via WinFsp/FUSE).

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
    last_scanned_letter: Option<char>,
}

impl UserSessionSearchIndex {
    pub fn new() -> Self {
        Self {
            volumes: HashMap::new(),
            watchers: HashMap::new(),
            last_discovery: None,
            last_scanned_letter: None,
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

        if let Some(candidate) =
            pick_next_stale_candidate(&stale_candidates, self.last_scanned_letter)
        {
            match scan_volume(candidate.drive_letter) {
                Ok(scan) => {
                    let count = scan.items.len();
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
                    self.last_scanned_letter = Some(candidate.drive_letter);
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

        self.volumes
            .retain(|letter, _| active_letters.contains(letter));
        self.watchers
            .retain(|letter, _| active_letters.contains(letter));
    }

    pub fn search(&self, query: &str, max_results: usize) -> Vec<SearchResultItem> {
        if query.is_empty() || max_results == 0 {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let mut results = Vec::with_capacity(max_results.min(128));

        for volume in self.volumes.values() {
            for item in &volume.items {
                if !volume.live_paths.contains(&item.path_key) {
                    continue;
                }

                if item.name_lower.contains(&query_lower) {
                    results.push(SearchResultItem {
                        name: item.name.clone(),
                        full_path: item.full_path.clone(),
                        is_dir: item.is_dir,
                        size: 0,
                    });

                    if results.len() >= max_results {
                        return results;
                    }
                }
            }
        }

        results
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

fn pick_next_stale_candidate(
    stale_candidates: &[CandidateVolume],
    last_scanned_letter: Option<char>,
) -> Option<&CandidateVolume> {
    if stale_candidates.is_empty() {
        return None;
    }

    let Some(last_letter) = last_scanned_letter else {
        return stale_candidates.first();
    };

    if let Some(next) = stale_candidates
        .iter()
        .find(|c| c.drive_letter > last_letter)
    {
        return Some(next);
    }

    stale_candidates.first()
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
