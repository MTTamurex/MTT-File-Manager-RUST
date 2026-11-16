//! Ordered normalized paths: misses and subtree queries never scan all assignments.
use crate::domain::file_tag::normalize_tag_path_key;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(crate) struct TagPathIndex {
    paths: BTreeMap<String, BTreeSet<PathBuf>>,
}

impl TagPathIndex {
    pub fn build(assignments: &FxHashMap<PathBuf, Vec<i64>>) -> Self {
        let mut index = Self::default();
        for path in assignments.keys() {
            index.insert(path.clone());
        }
        index
    }

    pub fn insert(&mut self, path: PathBuf) {
        self.paths
            .entry(normalize_tag_path_key(&path))
            .or_default()
            .insert(path);
    }

    pub fn remove(&mut self, path: &Path) {
        let key = normalize_tag_path_key(path);
        if let Some(paths) = self.paths.get_mut(&key) {
            paths.remove(path);
            if paths.is_empty() {
                self.paths.remove(&key);
            }
        }
    }

    pub fn exact(&self, path: &Path) -> impl Iterator<Item = &PathBuf> {
        self.paths
            .get(&normalize_tag_path_key(path))
            .into_iter()
            .flatten()
    }

    pub fn canonical(&self, path: &Path) -> Option<PathBuf> {
        let paths = self.paths.get(&normalize_tag_path_key(path))?;
        paths.get(path).or_else(|| paths.first()).cloned()
    }

    pub fn under_roots(&self, roots: &[PathBuf]) -> Vec<PathBuf> {
        let mut found = FxHashSet::default();
        for root in roots {
            let key = normalize_tag_path_key(root);
            if let Some(paths) = self.paths.get(&key) {
                found.extend(paths.iter().cloned());
            }
            let prefix = format!("{}\\", key.trim_end_matches('\\'));
            for (key, paths) in self.paths.range(prefix.clone()..) {
                if !key.starts_with(&prefix) {
                    break;
                }
                found.extend(paths.iter().cloned());
            }
        }
        found.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtree_queries_preserve_aliases_and_do_not_match_prefix_siblings() {
        let assignments = [
            (PathBuf::from(r"C:\Data\Folder\file.txt"), vec![1]),
            (PathBuf::from(r"c:\data\folder\FILE.txt"), vec![2]),
            (PathBuf::from(r"C:\Data\Folder-other\file.txt"), vec![3]),
            (PathBuf::from(r"D:\Other\file.txt"), vec![4]),
        ]
        .into_iter()
        .collect();
        let index = TagPathIndex::build(&assignments);
        assert_eq!(
            index
                .under_roots(&[PathBuf::from(r"\\?\C:\DATA\Folder")])
                .len(),
            2
        );
        assert_eq!(index.under_roots(&[PathBuf::from(r"C:\")]).len(), 3);
        assert!(index.canonical(Path::new(r"C:\Missing")).is_none());
        assert_eq!(
            index.exact(Path::new(r"C:/Data/Folder/file.txt")).count(),
            2
        );
    }

    #[test]
    fn untagged_batch_cost_does_not_depend_on_all_assignment_pairs() {
        let assignments = (0..20_000)
            .map(|n| (PathBuf::from(format!(r"D:\Tagged\file-{n}.txt")), vec![1]))
            .collect();
        let index = TagPathIndex::build(&assignments);
        let removed = (0..128)
            .map(|n| PathBuf::from(format!(r"C:\Other\file-{n}.txt")))
            .collect::<Vec<_>>();
        let started = std::time::Instant::now();
        assert!(index.under_roots(&removed).is_empty());
        eprintln!(
            "128 unrelated removals against 20000 tagged paths: {:?}",
            started.elapsed()
        );
    }
}
