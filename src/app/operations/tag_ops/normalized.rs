//! Case-insensitive, O(1)-lookup mirror of [`ImageViewerApp::tag_assignments`].
//!
//! `tag_assignments` keeps `PathBuf` keys (preserving original casing) so tag
//! mutations can detect and update an existing entry even when the filesystem
//! reports a different casing. Render code, however, runs `tag_ids_for_path`
//! once per visible item per frame; the previous `PathBuf::eq`-based fast path
//! silently fell back to an O(N) case-insensitive scan plus a `String`
//! allocation per comparison, which caused visible scroll lag in grid view when
//! many items had no tag (see `domain::file_tag::tag_ids_for_path`).
//!
//! To keep render O(1) without losing the casing tolerance needed for
//! mutations, we maintain `tag_assignments_normalized` — a parallel map keyed
//! by `normalize_tag_path_key(path)` (lowercased `\`-separated, no trailing
//! separator). It is rebuilt only when `tag_assignments` mutates, never per
//! frame.

use crate::app::state::ImageViewerApp;
use crate::domain::file_tag::normalize_tag_path_key;
use rustc_hash::FxHashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;
use std::sync::Arc;

/// Builds the normalized-key view of a `PathBuf -> Vec<i64>` assignments map.
///
/// Entries whose normalized keys collide (e.g. the same path stored twice with
/// different casing) are merged by extending the tag ID list; duplicates are
/// deduplicated to keep `path_has_tag` checks consistent.
pub(crate) fn build_tag_assignments_normalized(
    source: &FxHashMap<PathBuf, Vec<i64>>,
) -> FxHashMap<String, Vec<i64>> {
    let mut out: FxHashMap<String, Vec<i64>> =
        FxHashMap::with_capacity_and_hasher(source.len(), Default::default());
    for (path, ids) in source {
        let key = normalize_tag_path_key(path);
        match out.entry(key) {
            Entry::Occupied(mut occ) => {
                let existing = occ.get_mut();
                for id in ids {
                    if !existing.contains(id) {
                        existing.push(*id);
                    }
                }
            }
            Entry::Vacant(vac) => {
                vac.insert(ids.clone());
            }
        }
    }
    out
}

impl ImageViewerApp {
    /// Replaces `tag_assignments` and rebuilds the normalized mirror in one
    /// step. Prefer this over assigning `tag_assignments` directly so the two
    /// maps cannot drift.
    pub(crate) fn set_tag_assignments(&mut self, new: FxHashMap<PathBuf, Vec<i64>>) {
        let normalized = build_tag_assignments_normalized(&new);
        self.tag_assignments = Arc::new(new);
        self.tag_assignments_normalized = Arc::new(normalized);
    }

    /// Rebuilds `tag_assignments_normalized` from the current `tag_assignments`.
    /// Call this after any in-place mutation of `tag_assignments`
    /// (`Arc::make_mut` + retain/insert/remove) before the next render frame
    /// touches the normalized view.
    pub(crate) fn sync_tag_assignments_normalized(&mut self) {
        let normalized = build_tag_assignments_normalized(self.tag_assignments.as_ref());
        self.tag_assignments_normalized = Arc::new(normalized);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_tag::{path_has_tag, tag_ids_for_path};
    use std::path::Path;

    fn assignments_from(items: &[(&str, Vec<i64>)]) -> FxHashMap<PathBuf, Vec<i64>> {
        let mut m = FxHashMap::default();
        for (p, ids) in items {
            m.insert(PathBuf::from(p), ids.clone());
        }
        m
    }

    #[test]
    fn normalized_map_lowercases_and_merges_duplicate_casings() {
        let source = assignments_from(&[
            (r"C:\Users\Foo.txt", vec![1]),
            (r"c:\users\foo.txt", vec![2]),
            (r"C:/Users/Bar.txt/", vec![3]),
        ]);
        let normalized = build_tag_assignments_normalized(&source);

        // Normalized keys are lowercased, separator-normalized, no trailing `/`.
        let canonical = |p: &str| {
            tag_ids_for_path(&normalized, Path::new(p))
                .map(|s| s.to_vec())
                .unwrap_or_default()
        };
        // All casings collapse into one merged entry with deduped IDs.
        let merged = canonical(r"C:\Users\Foo.txt");
        merged.iter().for_each(|id| assert!(*id == 1 || *id == 2));
        assert_eq!(merged.len(), 2);
        // Different path key -> distinct entry.
        assert_eq!(canonical(r"C:\Users\Bar.txt"), vec![3]);
    }

    #[test]
    fn path_has_tag_uses_normalized_lookup() {
        let source = assignments_from(&[(r"C:\Path\With\Tag.txt", vec![7])]);
        let normalized = build_tag_assignments_normalized(&source);
        assert!(path_has_tag(
            &normalized,
            Path::new(r"c:\path\with\tag.txt"),
            7
        ));
        assert!(!path_has_tag(
            &normalized,
            Path::new(r"C:\Path\Untagged.txt"),
            7
        ));
    }
}
