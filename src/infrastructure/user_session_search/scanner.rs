use std::collections::{HashSet, VecDeque};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::infrastructure::drive_watcher::DriveWatcherEvent;

use super::{
    trigram, IndexedItem, IndexedVolume, FILE_ATTRIBUTE_REPARSE_POINT, MAX_ITEMS_PER_VOLUME,
};

pub(super) struct ScanOutcome {
    pub items: Vec<IndexedItem>,
    pub live_paths: HashSet<Arc<str>>,
    /// PERF-06: trigram index built over `items` (None above the size cap).
    pub trigram_index: Option<trigram::TrigramIndex>,
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

            // PERF-06: path_key is shared between the item and live_paths
            // (previously a full heap copy per path).
            let path_key: Arc<str> = Arc::from(normalize_path_key(&path));
            let is_dir = file_type.is_dir();
            let size = if !is_dir {
                entry.metadata().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };
            items.push(IndexedItem {
                name_lower: Arc::from(name.to_lowercase()),
                name: Arc::from(name),
                full_path: Arc::from(path.to_string_lossy().into_owned()),
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

    // PERF-06: build the trigram index on the scan thread (off the search
    // worker). Volumes above the cap keep the linear scan.
    let trigram_index = if items.len() <= trigram::TRIGRAM_INDEX_MAX_ITEMS {
        Some(trigram::TrigramIndex::build(
            items.iter().map(|item| &*item.name_lower),
        ))
    } else {
        log::debug!(
            "[SESSION-SEARCH] Volume {}:\\ has {} items (> {}); keeping linear scan",
            drive_letter,
            items.len(),
            trigram::TRIGRAM_INDEX_MAX_ITEMS
        );
        None
    };

    Ok(ScanOutcome {
        items,
        live_paths,
        trigram_index,
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
            volume
                .live_paths
                .remove(normalize_path_key(path).as_str());
        }
        DriveWatcherEvent::Renamed(old_path, new_path) => {
            volume
                .live_paths
                .remove(normalize_path_key(old_path).as_str());
            upsert_path(volume, new_path);
        }
        DriveWatcherEvent::PrefixInvalidated(prefix) => {
            invalidate_prefix(volume, prefix);
        }
        DriveWatcherEvent::Unknown(_) => {}
        DriveWatcherEvent::DriveLost(_) => {
            volume.live_paths.clear();
            volume.needs_rescan = true;
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
        path_key.as_ref() != prefix_key.as_str()
            && !path_key
                .strip_prefix(prefix_key.as_str())
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

    let key: Arc<str> = Arc::from(normalize_path_key(path));
    let full_path: Arc<str> = Arc::from(path.to_string_lossy().into_owned());
    let is_dir = crate::infrastructure::onedrive::fast_is_dir(path);
    let size = if !is_dir {
        std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let name_lower: Arc<str> = Arc::from(name.to_lowercase());

    let existing_idx = volume
        .items
        .iter()
        .position(|item| item.path_key.as_ref() == key.as_ref());

    if let Some(idx) = existing_idx {
        {
            let item = &mut volume.items[idx];
            item.name_lower = name_lower.clone();
            item.name = Arc::from(name);
            item.full_path = full_path;
            item.is_dir = is_dir;
            item.size = size;
        }
        volume.live_paths.insert(key);
        // PERF-06: keep the trigram index sound after a rename/modify.
        if let Some(index) = volume.trigram_index.as_mut() {
            index.insert_name(idx, &name_lower);
        }
        return;
    }

    volume.items.push(IndexedItem {
        name_lower: name_lower.clone(),
        name: Arc::from(name),
        full_path,
        path_key: key.clone(),
        is_dir,
        size,
    });
    volume.live_paths.insert(key);
    // PERF-06: index the newly appended item.
    if let Some(index) = volume.trigram_index.as_mut() {
        index.insert_name(volume.items.len() - 1, &name_lower);
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::sync::Arc;
    use std::time::Instant;

    use crate::infrastructure::user_session_search::{IndexedItem, IndexedVolume};

    use super::{normalize_path_key, upsert_path};

    #[test]
    fn upsert_path_refreshes_existing_file_size() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let file_path = dir.path().join("sample.txt");
        fs::write(&file_path, b"old").expect("write initial file");

        let key: Arc<str> = Arc::from(normalize_path_key(&file_path));
        let mut live_paths = HashSet::new();
        live_paths.insert(key.clone());
        let mut volume = IndexedVolume {
            label: String::new(),
            file_system: String::new(),
            last_scan: Instant::now(),
            items: vec![IndexedItem {
                name: Arc::from("sample.txt"),
                name_lower: Arc::from("sample.txt"),
                full_path: Arc::from(file_path.to_string_lossy().into_owned()),
                path_key: key,
                is_dir: false,
                size: 3,
            }],
            live_paths,
            needs_rescan: false,
            trigram_index: None,
        };

        fs::write(&file_path, b"new content").expect("rewrite file");
        upsert_path(&mut volume, &file_path);

        assert_eq!(volume.items.len(), 1);
        assert_eq!(volume.items[0].size, 11);
    }
}
