//! In-memory timestamp resolution shared by navigation and new-tab creation.

use crate::domain::file_entry::FileEntry;
use std::path::{Path, PathBuf};

pub(super) fn resolve_modified_hint(
    destination: &Path,
    items: &[FileEntry],
    selected: Option<&FileEntry>,
    current: Option<&(PathBuf, u64)>,
    cached: Option<u64>,
) -> Option<(PathBuf, u64)> {
    items
        .iter()
        .find(|item| item.is_dir && item.path == destination && item.modified > 0)
        .map(|item| (item.path.clone(), item.modified))
        .or_else(|| {
            selected
                .filter(|item| item.is_dir && item.path == destination && item.modified > 0)
                .map(|item| (item.path.clone(), item.modified))
        })
        .or_else(|| {
            current
                .filter(|(path, value)| path == destination && *value > 0)
                .cloned()
        })
        .or_else(|| {
            cached
                .filter(|value| *value > 0)
                .map(|value| (destination.to_path_buf(), value))
        })
}

pub(super) fn resolve_created_hint(
    destination: &Path,
    items: &[FileEntry],
    selected: Option<&FileEntry>,
    current: Option<&(PathBuf, u64)>,
    cached: Option<u64>,
) -> Option<(PathBuf, u64)> {
    items
        .iter()
        .find(|item| item.is_dir && item.path == destination)
        .and_then(|item| {
            item.created
                .filter(|value| *value > 0)
                .map(|value| (item.path.clone(), value))
        })
        .or_else(|| {
            selected
                .filter(|item| item.is_dir && item.path == destination)
                .and_then(|item| {
                    item.created
                        .filter(|value| *value > 0)
                        .map(|value| (item.path.clone(), value))
                })
        })
        .or_else(|| {
            current
                .filter(|(path, value)| path == destination && *value > 0)
                .cloned()
        })
        .or_else(|| {
            cached
                .filter(|value| *value > 0)
                .map(|value| (destination.to_path_buf(), value))
        })
}

pub(super) fn needs_resolution(
    destination: &Path,
    modified: Option<&(PathBuf, u64)>,
    created: Option<&(PathBuf, u64)>,
) -> bool {
    let has_modified = modified.is_some_and(|(path, value)| path == destination && *value > 0);
    let has_created = created.is_some_and(|(path, value)| path == destination && *value > 0);
    !has_modified || !has_created
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::SyncStatus;

    fn folder(path: &str, modified: u64, created: Option<u64>) -> FileEntry {
        FileEntry {
            path: PathBuf::from(path),
            name: "Folder".to_owned(),
            is_dir: true,
            size: 0,
            modified,
            created,
            folder_cover: None,
            drive_info: None,
            sync_status: SyncStatus::None,
            is_hidden: false,
            recycle_bin: None,
        }
    }

    #[test]
    fn unvisited_folder_uses_source_listing_without_cached_timestamps() {
        let destination = Path::new(r"Z:\Folder");
        let items = [folder(r"Z:\Folder", 200, Some(100))];
        let modified = resolve_modified_hint(destination, &items, None, None, None);
        let created = resolve_created_hint(destination, &items, None, None, None);
        assert_eq!(modified, Some((destination.to_path_buf(), 200)));
        assert_eq!(created, Some((destination.to_path_buf(), 100)));
        assert!(!needs_resolution(
            destination,
            modified.as_ref(),
            created.as_ref()
        ));
    }

    #[test]
    fn selected_folder_supplies_missing_listing_timestamps() {
        let destination = Path::new(r"C:\Folder");
        let items = [folder(r"C:\Folder", 0, None)];
        let selected = folder(r"C:\Folder", 200, Some(100));
        assert_eq!(
            resolve_modified_hint(destination, &items, Some(&selected), None, None),
            Some((destination.to_path_buf(), 200))
        );
        assert_eq!(
            resolve_created_hint(destination, &items, Some(&selected), None, None),
            Some((destination.to_path_buf(), 100))
        );
    }

    #[test]
    fn same_folder_new_tab_reuses_current_hints_without_cache() {
        let destination = Path::new(r"C:\Folder");
        let modified = (destination.to_path_buf(), 200);
        let created = (destination.to_path_buf(), 100);
        assert_eq!(
            resolve_modified_hint(destination, &[], None, Some(&modified), None),
            Some(modified)
        );
        assert_eq!(
            resolve_created_hint(destination, &[], None, Some(&created), None),
            Some(created)
        );
    }

    #[test]
    fn unrelated_folder_and_file_entries_cannot_supply_timestamps() {
        let destination = Path::new(r"C:\Folder");
        let unrelated = folder(r"D:\Folder", 300, Some(150));
        let mut file = folder(r"C:\Folder", 200, Some(100));
        file.is_dir = false;
        let items = [file, unrelated.clone()];
        let wrong_hint = (unrelated.path.clone(), 400);
        assert!(resolve_modified_hint(
            destination,
            &items,
            Some(&unrelated),
            Some(&wrong_hint),
            None
        )
        .is_none());
        assert!(resolve_created_hint(
            destination,
            &items,
            Some(&unrelated),
            Some(&wrong_hint),
            None
        )
        .is_none());
        assert!(needs_resolution(
            destination,
            Some(&wrong_hint),
            Some(&wrong_hint)
        ));
    }

    #[test]
    fn cache_fills_missing_values_but_source_listing_takes_precedence() {
        let destination = Path::new(r"C:\Folder");
        let items = [folder(r"C:\Folder", 200, None)];
        assert_eq!(
            resolve_modified_hint(destination, &items, None, None, Some(300)),
            Some((destination.to_path_buf(), 200))
        );
        assert_eq!(
            resolve_created_hint(destination, &items, None, None, Some(100)),
            Some((destination.to_path_buf(), 100))
        );
    }

    #[test]
    fn background_resolution_is_needed_if_either_date_is_missing_or_zero() {
        let destination = Path::new(r"C:\Folder");
        let hint = (destination.to_path_buf(), 100);
        let zero = (destination.to_path_buf(), 0);
        assert!(needs_resolution(destination, None, None));
        assert!(needs_resolution(destination, Some(&hint), None));
        assert!(needs_resolution(destination, None, Some(&hint)));
        assert!(needs_resolution(destination, Some(&hint), Some(&zero)));
        assert!(needs_resolution(destination, Some(&zero), Some(&hint)));
        assert!(!needs_resolution(destination, Some(&hint), Some(&hint)));
        assert!(resolve_modified_hint(destination, &[], None, Some(&zero), Some(0)).is_none());
        assert!(resolve_created_hint(destination, &[], None, Some(&zero), Some(0)).is_none());
    }
}
