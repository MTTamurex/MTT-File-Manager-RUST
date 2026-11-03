use super::*;
use std::collections::VecDeque;

const SCAN_MAX_DEPTH: usize = 32;

/// Includes the starting paths, then their children without following reparse
/// points. A shutdown request interrupts traversal, including explicit scans.
pub(super) fn collect_paths(
    context: &AutoTagContext,
    roots: Vec<PathBuf>,
    recursive: bool,
) -> Vec<PathBuf> {
    let mut queue: VecDeque<_> = roots.into_iter().map(|path| (path, 0)).collect();
    let mut seen = FxHashSet::default();
    let mut paths = Vec::new();
    while let Some((path, depth)) = queue.pop_front() {
        if context.shutdown.load(Ordering::Acquire) {
            break;
        }
        if !seen.insert(crate::domain::file_tag::normalize_tag_path_key(&path)) {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x0400 != 0 {
            continue;
        }
        if metadata.is_dir() && depth < if recursive { SCAN_MAX_DEPTH } else { 1 } {
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.flatten() {
                    if context.shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    queue.push_back((entry.path(), depth + 1));
                }
            }
        }
        paths.push(path);
    }
    paths
}

pub(super) fn scan_and_apply(
    context: &AutoTagContext,
    rules: &[AutoTagRule],
    defs_by_id: &FxHashMap<i64, FileTag>,
    rule_id: Option<i64>,
) {
    let mut paths_by_tag: FxHashMap<i64, FxHashSet<PathBuf>> = FxHashMap::default();
    for rule in rules
        .iter()
        .filter(|rule| rule.enabled && rule_id.is_none_or(|id| id == rule.id))
    {
        for path in collect_paths(
            context,
            vec![rule.folder_path.clone()],
            rule.include_subfolders,
        ) {
            // Match only this rule: overlapping rules should not cause an
            // explicit "apply now" to apply unrelated tags or tag their roots.
            if path == rule.folder_path || (!rule.include_subfolders && path.is_dir()) {
                continue;
            }
            paths_by_tag.entry(rule.tag_id).or_default().insert(path);
        }
    }
    let mut assignments = Vec::new();
    for (tag_id, paths) in paths_by_tag {
        if context.shutdown.load(Ordering::Acquire) {
            break;
        }
        if !defs_by_id.contains_key(&tag_id) {
            continue;
        }
        let paths: Vec<_> = paths.into_iter().collect();
        let chain = tags_to_assign(defs_by_id, tag_id);
        if context.app_state_db.assign_tags_batch(&paths, &chain) {
            assignments.extend(paths.into_iter().map(|path| (path, tag_id)));
        } else {
            log::warn!("[AUTO-TAG] Scan failed to assign tag {tag_id}");
        }
    }
    if !assignments.is_empty() {
        let _ = context
            .event_sender
            .send(AutoTagEvent::Applied { assignments });
    }
}
