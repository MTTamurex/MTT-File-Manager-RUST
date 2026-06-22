use crate::domain::file_tag::FileTag;
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};

mod assignments;
mod cache;
mod definitions;
pub(crate) mod normalized;
mod view;

fn normalize_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn path_is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    let candidate = normalize_path_text(candidate);
    let root = normalize_path_text(root);
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn remap_path(candidate: &Path, old_root: &Path, new_root: &Path) -> PathBuf {
    if let Ok(suffix) = candidate.strip_prefix(old_root) {
        return new_root.join(suffix);
    }

    let candidate = candidate.to_string_lossy();
    let old_root = old_root.to_string_lossy();
    let new_root = new_root.to_string_lossy();
    let old_root = old_root.trim_end_matches(['\\', '/']);
    if candidate.len() <= old_root.len() {
        return PathBuf::from(new_root.as_ref());
    }

    let suffix = &candidate[old_root.len()..];
    PathBuf::from(format!(
        "{}{}",
        new_root.trim_end_matches(['\\', '/']),
        suffix
    ))
}

fn tag_sort_key(tag: &FileTag) -> (i64, String) {
    (tag.position, tag.name.to_lowercase())
}

fn tag_assignment_path_matches(assigned_path: &Path, path: &Path) -> bool {
    normalize_path_text(assigned_path) == normalize_path_text(path)
}

fn tag_assignment_key_for_path<'a>(
    assignments: &'a FxHashMap<PathBuf, Vec<i64>>,
    path: &Path,
) -> Option<&'a PathBuf> {
    if let Some((key, _)) = assignments.get_key_value(path) {
        return Some(key);
    }

    assignments
        .keys()
        .find(|assigned_path| tag_assignment_path_matches(assigned_path, path))
}

fn tag_assignment_key_for_path_with_tag<'a>(
    assignments: &'a FxHashMap<PathBuf, Vec<i64>>,
    path: &Path,
    tag_id: i64,
) -> Option<&'a PathBuf> {
    if let Some((key, ids)) = assignments.get_key_value(path) {
        if ids.contains(&tag_id) {
            return Some(key);
        }
    }

    assignments.iter().find_map(|(assigned_path, ids)| {
        (ids.contains(&tag_id) && tag_assignment_path_matches(assigned_path, path))
            .then_some(assigned_path)
    })
}
