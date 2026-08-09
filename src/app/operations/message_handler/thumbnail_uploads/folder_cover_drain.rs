use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;
use std::collections::HashSet;
use std::path::PathBuf;

pub(super) struct FolderCoverThumbnailUploadIndex {
    item_paths: FxHashSet<PathBuf>,
    cover_paths: FxHashSet<PathBuf>,
}

impl FolderCoverThumbnailUploadIndex {
    pub(super) fn new(
        active_items: &[FileEntry],
        all_active_items: &[FileEntry],
        inactive_items: Option<&[FileEntry]>,
    ) -> Self {
        let mut item_paths = FxHashSet::default();
        item_paths.reserve(
            active_items.len()
                + all_active_items.len()
                + inactive_items.map_or(0, <[FileEntry]>::len),
        );
        item_paths.extend(active_items.iter().map(|item| item.path.clone()));
        item_paths.extend(all_active_items.iter().map(|item| item.path.clone()));

        let mut cover_paths = FxHashSet::default();
        cover_paths.extend(all_active_items.iter().filter_map(|item| {
            item.is_dir
                .then_some(item.folder_cover.as_ref())
                .flatten()
                .cloned()
        }));

        if let Some(inactive_items) = inactive_items {
            item_paths.extend(inactive_items.iter().map(|item| item.path.clone()));
            cover_paths.extend(inactive_items.iter().filter_map(|item| {
                item.is_dir
                    .then_some(item.folder_cover.as_ref())
                    .flatten()
                    .cloned()
            }));
        }

        Self {
            item_paths,
            cover_paths,
        }
    }

    pub(super) fn should_skip(&self, path: &PathBuf, selected_path: Option<&PathBuf>) -> bool {
        selected_path != Some(path)
            && !self.item_paths.contains(path)
            && self.cover_paths.contains(path)
    }
}

pub(super) struct FolderCoverFollowupIndex {
    uncovered_folders: FxHashSet<PathBuf>,
    covered_folders: Vec<(PathBuf, PathBuf)>,
}

impl FolderCoverFollowupIndex {
    pub(super) fn new(all_active_items: &[FileEntry]) -> Self {
        let mut uncovered_folders = FxHashSet::default();
        let mut covered_folders = Vec::new();

        for item in all_active_items.iter().filter(|item| item.is_dir) {
            if let Some(cover_path) = item.folder_cover.as_ref() {
                covered_folders.push((item.path.clone(), cover_path.clone()));
            } else {
                uncovered_folders.insert(item.path.clone());
            }
        }

        Self {
            uncovered_folders,
            covered_folders,
        }
    }
}

pub(super) struct FolderCoverFollowups {
    pub(super) folders_to_scan: Vec<PathBuf>,
    pub(super) previews_to_recompose: Vec<(PathBuf, PathBuf)>,
}

pub(super) fn folder_cover_followups(
    successful_thumb_paths: &[PathBuf],
    index: &FolderCoverFollowupIndex,
) -> FolderCoverFollowups {
    let successful_set: HashSet<&PathBuf> = successful_thumb_paths.iter().collect();
    let mut seen_scan_folders = FxHashSet::default();
    let mut folders_to_scan = Vec::new();

    for thumb_path in successful_thumb_paths {
        let Some(parent) = thumb_path.parent() else {
            continue;
        };
        if index.uncovered_folders.contains(parent) {
            let parent = parent.to_path_buf();
            if seen_scan_folders.insert(parent.clone()) {
                folders_to_scan.push(parent);
            }
        }
    }

    let previews_to_recompose = index
        .covered_folders
        .iter()
        .filter(|(_, cover_path)| successful_set.contains(cover_path))
        .cloned()
        .collect();

    FolderCoverFollowups {
        folders_to_scan,
        previews_to_recompose,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::SyncStatus;

    fn entry(path: &str, is_dir: bool, cover: Option<&str>) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            name: String::new(),
            is_dir,
            size: 0,
            modified: 0,
            created: None,
            folder_cover: cover.map(PathBuf::from),
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn upload_index_preserves_active_all_and_inactive_semantics() {
        let active_items = [entry("active/visible.jpg", false, None)];
        let all_active_items = [
            entry("active/hidden.jpg", false, None),
            entry("active/folder", true, Some("active/folder/cover.jpg")),
        ];
        let inactive_items = [
            entry("inactive/visible.jpg", false, None),
            entry("inactive/folder", true, Some("inactive/folder/cover.jpg")),
        ];
        let index = FolderCoverThumbnailUploadIndex::new(
            &active_items,
            &all_active_items,
            Some(&inactive_items),
        );

        for item_path in [
            "active/visible.jpg",
            "active/hidden.jpg",
            "inactive/visible.jpg",
        ] {
            assert!(!index.should_skip(&PathBuf::from(item_path), None));
        }
        assert!(index.should_skip(&PathBuf::from("active/folder/cover.jpg"), None));
        assert!(index.should_skip(&PathBuf::from("inactive/folder/cover.jpg"), None));
        assert!(!index.should_skip(&PathBuf::from("unrelated.jpg"), None));

        let selected = PathBuf::from("active/folder/cover.jpg");
        assert!(!index.should_skip(&selected, Some(&selected)));
    }

    #[test]
    fn followups_preserve_success_and_all_items_order() {
        let all_active_items = [
            entry("root/folder-b", true, Some("root/folder-b/cover.jpg")),
            entry("root/uncovered", true, None),
            entry("root/folder-a", true, Some("root/folder-a/cover.jpg")),
            entry("root/file.jpg", false, None),
        ];
        let successful = vec![
            PathBuf::from("root/uncovered/first.jpg"),
            PathBuf::from("root/folder-a/cover.jpg"),
            PathBuf::from("root/folder-b/cover.jpg"),
            PathBuf::from("root/uncovered/second.jpg"),
        ];
        let index = FolderCoverFollowupIndex::new(&all_active_items);

        let followups = folder_cover_followups(&successful, &index);

        assert_eq!(
            followups.folders_to_scan,
            vec![PathBuf::from("root/uncovered")]
        );
        assert_eq!(
            followups.previews_to_recompose,
            vec![
                (
                    PathBuf::from("root/folder-b"),
                    PathBuf::from("root/folder-b/cover.jpg")
                ),
                (
                    PathBuf::from("root/folder-a"),
                    PathBuf::from("root/folder-a/cover.jpg")
                ),
            ]
        );
    }
}
