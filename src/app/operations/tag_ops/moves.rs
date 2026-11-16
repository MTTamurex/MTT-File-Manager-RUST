//! Resolve a whole move batch against the pre-move snapshot, then publish once.
use super::persistence::TagWrite;
use super::*;
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use std::collections::BTreeMap;
use std::sync::Arc;

struct PathMoves(BTreeMap<PathBuf, PathBuf>);

impl PathMoves {
    fn new(pairs: &[(PathBuf, PathBuf)]) -> Self {
        Self(
            pairs
                .iter()
                .map(|(old, new)| (PathBuf::from(normalize_path_text(old)), new.clone()))
                .collect(),
        )
    }

    fn remap(&self, path: &Path) -> Option<PathBuf> {
        let key = PathBuf::from(normalize_path_text(path));
        for ancestor in key.ancestors() {
            if let Some(target) = self.0.get(ancestor) {
                let original = PathBuf::from(crate::domain::file_tag::normalize_tag_path_text(
                    &path.to_string_lossy(),
                ));
                let suffix: PathBuf = original
                    .components()
                    .skip(ancestor.components().count())
                    .collect();
                return Some(target.join(suffix));
            }
        }
        None
    }

    fn remap_entry(&self, entry: &mut FileEntry) {
        if let Some(path) = self.remap(&entry.path) {
            entry.path = path;
            if let Some(name) = entry.path.file_name() {
                entry.name = name.to_string_lossy().into_owned();
            }
        }
        if let Some(cover) = &mut entry.folder_cover {
            if let Some(path) = self.remap(cover) {
                *cover = path;
            }
        }
    }
}

impl ImageViewerApp {
    pub fn move_tag_assignments_for_path(&mut self, old: &Path, new: &Path) {
        self.move_tag_assignments_for_paths(&[(old.to_path_buf(), new.to_path_buf())]);
    }

    pub fn move_tag_assignments_for_paths(&mut self, pairs: &[(PathBuf, PathBuf)]) {
        let pairs: Vec<_> = pairs
            .iter()
            .filter(|(old, new)| old != new)
            .cloned()
            .collect();
        let roots: Vec<_> = pairs.iter().map(|(old, _)| old.clone()).collect();
        let sources = self.tag_path_index.under_roots(&roots);
        if sources.is_empty() {
            return;
        }
        let remapper = PathMoves::new(&pairs);
        let source_set: FxHashSet<_> = sources.iter().cloned().collect();
        let mut changes: FxHashMap<PathBuf, Vec<i64>> = sources
            .iter()
            .map(|path| (path.clone(), Vec::new()))
            .collect();
        let mut db_moves = Vec::new();
        for source in &sources {
            let Some(target) = remapper.remap(source) else {
                continue;
            };
            let target = self
                .tag_path_index
                .canonical(&target)
                .filter(|existing| normalize_path_text(existing) != normalize_path_text(source))
                .unwrap_or(target);
            let ids = &self.tag_assignments[source];
            let assigned = changes.entry(target.clone()).or_insert_with(|| {
                if source_set.contains(&target) {
                    Vec::new()
                } else {
                    self.tag_assignments
                        .get(&target)
                        .cloned()
                        .unwrap_or_default()
                }
            });
            for id in ids {
                db_moves.push((source.clone(), target.clone(), *id));
                if !assigned.contains(id) {
                    assigned.push(*id);
                }
            }
        }
        if !self.queue_tag_write(TagWrite::Move(db_moves)) {
            return;
        }
        self.replace_tag_paths(changes.into_iter().collect());
        let diverged = !Arc::ptr_eq(&self.items, &self.all_items);
        for entry in Arc::make_mut(&mut self.all_items) {
            remapper.remap_entry(entry);
        }
        if diverged {
            for entry in Arc::make_mut(&mut self.items) {
                remapper.remap_entry(entry);
            }
        } else {
            self.share_visible_items_from_all_items();
        }
        if let Some(panel) = &mut self.dual_panel_inactive_state {
            for entry in Arc::make_mut(&mut panel.all_items) {
                remapper.remap_entry(entry);
            }
            if !panel.items_snapshot_compact {
                for entry in Arc::make_mut(&mut panel.items) {
                    remapper.remap_entry(entry);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn batch_remapping_is_component_scoped_and_prefers_the_deepest_root() {
        let moves = PathMoves::new(&[
            (
                PathBuf::from(r"C:\Data\Folder"),
                PathBuf::from(r"D:\Target"),
            ),
            (
                PathBuf::from(r"C:\Data\Folder\Child"),
                PathBuf::from(r"D:\Other"),
            ),
        ]);
        assert_eq!(
            moves.remap(Path::new(r"\\?\C:\DATA\Folder\Child\file.txt")),
            Some(PathBuf::from(r"D:\Other\file.txt"))
        );
        assert!(moves
            .remap(Path::new(r"C:\Data\Folder-other\file.txt"))
            .is_none());
    }
}
