use std::collections::HashMap;

use crate::file_index::VolumeIndex;

/// The NTFS root directory's File Reference Number.
const NTFS_ROOT_FRN: u64 = 5;

/// Maximum depth for path resolution (safety limit).
const MAX_DEPTH: usize = 256;

/// Reconstruct full path with directory path caching.
///
/// The `dir_cache` maps directory FRN → full directory path string.
/// This avoids redundant parent-chain walks when many files share parent
/// directories, which is the common case during search result resolution.
pub fn resolve_path_cached(
    frn: u64,
    index: &VolumeIndex,
    dir_cache: &mut HashMap<u64, String>,
) -> Option<String> {
    let record = index.records.get(&frn)?;
    let file_name = index.names.get(record.name_ref());

    if frn == NTFS_ROOT_FRN {
        return None;
    }

    let parent_frn = record.parent_ref;

    // Self-referencing or zero parent: treat as top-level item
    if parent_frn == frn || parent_frn == 0 {
        return Some(format!("{}:\\{}", index.drive_letter, file_name));
    }

    // Parent is root: direct child of volume root
    if parent_frn == NTFS_ROOT_FRN {
        return Some(format!("{}:\\{}", index.drive_letter, file_name));
    }

    // Fast path: parent directory path is already cached
    if let Some(parent_path) = dir_cache.get(&parent_frn) {
        return Some(format!("{}\\{}", parent_path, file_name));
    }

    // Slow path: walk parent chain and cache intermediate directory paths
    let parent_path = resolve_dir_path(parent_frn, index, dir_cache)?;

    Some(format!("{}\\{}", parent_path, file_name))
}

/// Resolve the full path of a directory by walking its parent chain.
/// Caches all intermediate directory paths for future lookups.
fn resolve_dir_path(
    dir_frn: u64,
    index: &VolumeIndex,
    cache: &mut HashMap<u64, String>,
) -> Option<String> {
    if let Some(cached) = cache.get(&dir_frn) {
        return Some(cached.clone());
    }

    if dir_frn == NTFS_ROOT_FRN || dir_frn == 0 {
        return Some(format!("{}:", index.drive_letter));
    }

    // Collect uncached ancestors walking toward root
    let mut chain: Vec<(u64, &str)> = Vec::with_capacity(16);
    let mut current = dir_frn;

    for _ in 0..MAX_DEPTH {
        if current == NTFS_ROOT_FRN || current == 0 {
            break;
        }

        if cache.contains_key(&current) {
            break;
        }

        let rec = index.records.get(&current)?;
        chain.push((current, index.names.get(rec.name_ref())));

        let parent = rec.parent_ref;
        if parent == current {
            break;
        }
        current = parent;
    }

    // Start from the deepest cached ancestor or volume root
    let mut path = if let Some(cached) = cache.get(&current) {
        cached.clone()
    } else {
        format!("{}:", index.drive_letter)
    };

    // Build and cache paths from root downward
    chain.reverse();
    for (ancestor_frn, ancestor_name) in chain {
        path = format!("{}\\{}", path, ancestor_name);
        cache.insert(ancestor_frn, path.clone());
    }

    Some(path)
}
