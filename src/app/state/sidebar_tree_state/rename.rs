//! Preserve visible branches when a folder is renamed successfully.

use super::*;

fn remap(path: &Path, old: &Path, new: &Path) -> PathBuf {
    path.strip_prefix(old)
        .map(|suffix| new.join(suffix))
        .unwrap_or_else(|_| path.to_path_buf())
}

impl SidebarTreeState {
    pub(crate) fn remap_renamed_folder(&mut self, old: &Path, new: &Path) {
        if !self.expanded.iter().any(|path| path.starts_with(old))
            && !self.children.keys().any(|path| path.starts_with(old))
        {
            return;
        }
        self.expanded = self
            .expanded
            .drain()
            .map(|path| remap(&path, old, new))
            .collect();
        self.children = std::mem::take(&mut self.children)
            .into_iter()
            .map(|(path, mut nodes)| {
                for node in &mut nodes {
                    if node.path == old {
                        if let Some(name) = new.file_name() {
                            node.name = name.to_string_lossy().into_owned();
                        }
                    }
                    node.path = remap(&node.path, old, new);
                }
                (remap(&path, old, new), nodes)
            })
            .collect();
        let reload: Vec<_> = self
            .loading
            .iter()
            .filter(|path| path.starts_with(old))
            .map(|path| remap(path, old, new))
            .collect();
        self.loading.retain(|path| !path.starts_with(old));
        for path in reload {
            if self.expanded.contains(&path) {
                self.load_children(&path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_tree_rename_preserves_descendants_and_unrelated_branches() {
        let mut tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
        let old = Path::new(r"C:\Folder");
        let new = Path::new(r"C:\Renamed");
        let child = old.join("Child");
        let other = PathBuf::from(r"C:\Other");
        tree.expanded
            .extend([old.into(), child.clone(), other.clone()]);
        tree.children.insert(
            old.into(),
            vec![FolderNode {
                path: child,
                name: "Child".into(),
                has_subfolders: Some(true),
                is_hidden: false,
            }],
        );
        tree.remap_renamed_folder(old, new);
        assert!(tree.is_expanded(new));
        assert!(tree.is_expanded(&new.join("Child")));
        assert!(tree.is_expanded(&other));
        assert!(!tree.is_expanded(old));
        assert_eq!(tree.get_children(new).unwrap()[0].path, new.join("Child"));
        assert!(tree.get_children(old).is_none());

        tree.tx
            .send(LoadResult {
                parent: old.into(),
                children: Some(Vec::new()),
            })
            .unwrap();
        assert!(
            !tree.poll_loaded(),
            "a superseded load must not revive the old branch"
        );
        assert!(tree.is_expanded(new));
        assert!(tree.get_children(old).is_none());
    }
}
