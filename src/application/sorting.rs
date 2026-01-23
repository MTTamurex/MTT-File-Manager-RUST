use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode};
use rayon::prelude::*;
use std::cmp::Ordering;

/// Sorts a slice of FileEntry in place based on the given criteria.
/// Uses Rayon for parallel sorting if the list is large (>5000 items).
pub fn sort_items(
    items: &mut [FileEntry],
    mode: SortMode,
    descending: bool,
    folders_position: FoldersPosition,
) {
    // Helper to check if an item is a "true" directory (not a ZIP file)
    let is_true_dir = |item: &FileEntry| -> bool {
        item.is_dir && !item.name.to_lowercase().ends_with(".zip")
    };

    let compare = |a: &FileEntry, b: &FileEntry| -> Ordering {
        // 1. Folders Position logic (ZIP files should be treated as files, not folders)
        let a_is_dir = is_true_dir(a);
        let b_is_dir = is_true_dir(b);
        if folders_position != FoldersPosition::Mixed && a_is_dir != b_is_dir {
            let folders_come_first = folders_position == FoldersPosition::First;
            return if a_is_dir {
                if folders_come_first {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            } else {
                if folders_come_first {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            };
        }

        // 2. Primary sort criteria
        let ordering = match mode {
            SortMode::Name => natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase()),
            SortMode::Date => a.modified.cmp(&b.modified),
            SortMode::Size => a.size.cmp(&b.size),
            SortMode::Type => {
                let ext_a = a
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let ext_b = b
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                match ext_a.cmp(&ext_b) {
                    Ordering::Equal => {
                        natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase())
                    }
                    other => other,
                }
            }
        };

        // 3. Apply Descending direction
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    };

    // Adaptive threshold
    const PARALLEL_THRESHOLD: usize = 5000;

    if items.len() > PARALLEL_THRESHOLD {
        items.par_sort_by(compare);
    } else {
        items.sort_by(compare);
    }
}

/// Filters items based on a query string. Returns a new Vec of matching items.
pub fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry> {
    if query.is_empty() {
        return items.to_vec();
    }

    let lower_query = query.to_lowercase();
    items
        .iter()
        .filter(|item| item.name.to_lowercase().contains(&lower_query))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_test_file(name: &str, size: u64, modified: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(name),
            name: name.to_string(),
            is_dir: false,
            size,
            modified,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            deletion_date: None,
            recycle_original_path: None,
        }
    }

    fn create_test_dir(name: &str, modified: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(name),
            name: name.to_string(),
            is_dir: true,
            size: 0,
            modified,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            deletion_date: None,
            recycle_original_path: None,
        }
    }

    #[test]
    fn test_sort_by_name_natural() {
        let mut items = vec![
            create_test_file("file10.txt", 0, 0),
            create_test_file("file2.txt", 0, 0),
            create_test_file("file1.txt", 0, 0),
        ];

        sort_items(&mut items, SortMode::Name, false, FoldersPosition::Mixed);
        assert_eq!(items[0].name, "file1.txt");
        assert_eq!(items[1].name, "file2.txt");
        assert_eq!(items[2].name, "file10.txt");
    }

    #[test]
    fn test_sort_by_size_descending() {
        let mut items = vec![
            create_test_file("small.txt", 100, 0),
            create_test_file("large.txt", 1000, 0),
            create_test_file("medium.txt", 500, 0),
        ];

        sort_items(&mut items, SortMode::Size, true, FoldersPosition::Mixed);
        assert_eq!(items[0].name, "large.txt");
        assert_eq!(items[1].name, "medium.txt");
        assert_eq!(items[2].name, "small.txt");
    }

    #[test]
    fn test_folders_first() {
        let mut items = vec![
            create_test_file("z_file.txt", 0, 0),
            create_test_dir("a_dir", 0),
            create_test_file("a_file.txt", 0, 0),
        ];

        sort_items(&mut items, SortMode::Name, false, FoldersPosition::First);
        assert_eq!(items[0].name, "a_dir");
        assert_eq!(items[1].name, "a_file.txt");
        assert_eq!(items[2].name, "z_file.txt");
    }

    #[test]
    fn test_folders_last() {
        let mut items = vec![
            create_test_dir("z_dir", 0),
            create_test_file("a_file.txt", 0, 0),
            create_test_dir("a_dir", 0),
        ];

        sort_items(&mut items, SortMode::Name, false, FoldersPosition::Last);
        assert_eq!(items[0].name, "a_file.txt");
        assert_eq!(items[1].name, "a_dir");
        assert_eq!(items[2].name, "z_dir");
    }

    #[test]
    fn test_filter_items() {
        let items = vec![
            create_test_file("apple.txt", 0, 0),
            create_test_file("banana.txt", 0, 0),
            create_test_file("pineapple.txt", 0, 0),
        ];

        let filtered = filter_items(&items, "apple");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].name, "apple.txt");
        assert_eq!(filtered[1].name, "pineapple.txt");

        let filtered_empty = filter_items(&items, "");
        assert_eq!(filtered_empty.len(), 3);
    }
}
