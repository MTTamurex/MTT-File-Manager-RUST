use std::collections::{HashSet, VecDeque};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::infrastructure::drive_watcher::DriveWatcherEvent;

use super::{IndexedItem, IndexedVolume, FILE_ATTRIBUTE_REPARSE_POINT, MAX_ITEMS_PER_VOLUME};

pub(super) struct ScanOutcome {
    pub items: Vec<IndexedItem>,
    pub live_paths: HashSet<String>,
    pub directories_scanned: usize,
    pub errors: usize,
    pub elapsed: std::time::Duration,
}

pub(super) fn scan_volume(drive_letter: char) -> Result<ScanOutcome, String> {
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
            let size = if !is_dir {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            items.push(IndexedItem {
                name_lower: name.to_lowercase(),
                name,
                full_path: path.to_string_lossy().into_owned(),
                path_key: path_key.clone(),
                is_dir,
                size,
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

pub(super) fn apply_event_to_volume(volume: &mut IndexedVolume, event: &DriveWatcherEvent) {
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
        DriveWatcherEvent::PrefixInvalidated(prefix) => {
            invalidate_prefix(volume, prefix);
        }
        DriveWatcherEvent::Unknown(_) => {}
        DriveWatcherEvent::DriveLost(_) => {
            volume.live_paths.clear();
        }
    }
}

fn invalidate_prefix(volume: &mut IndexedVolume, prefix: &Path) {
    let prefix_key = normalize_path_key(prefix);
    if prefix_key.len() <= 3 {
        volume.live_paths.clear();
        return;
    }

    volume.live_paths.retain(|path_key| {
        path_key != &prefix_key
            && !path_key
                .strip_prefix(&prefix_key)
                .is_some_and(|suffix| suffix.starts_with('\\'))
    });
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
    let is_dir = crate::infrastructure::onedrive::fast_is_dir(path);
    let size = if !is_dir {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    volume.items.push(IndexedItem {
        name_lower: name.to_lowercase(),
        name,
        full_path,
        path_key: key.clone(),
        is_dir,
        size,
    });
    volume.live_paths.insert(key);
}

pub(super) fn normalize_path_key(path: &Path) -> String {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let stripped = lower.strip_prefix(r"\\?\").unwrap_or(&lower);

    if stripped.len() > 3 {
        stripped.trim_end_matches('\\').to_string()
    } else {
        stripped.to_string()
    }
}
