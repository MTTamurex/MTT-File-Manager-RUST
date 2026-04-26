use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode};

mod filtering;
mod sort_impl;

/// Sorts a slice of FileEntry in place based on the given criteria.
/// Uses Rayon for parallel sorting if the list is large (>5000 items).
///
/// PERFORMANCE: Uses zero-allocation case-insensitive comparisons.
pub fn sort_items(
    items: &mut [FileEntry],
    mode: SortMode,
    descending: bool,
    folders_position: FoldersPosition,
) {
    sort_impl::sort_items(items, mode, descending, folders_position)
}

/// Filters items based on a query string.
///
/// PERFORMANCE: When query is empty, returns None to signal "use all items"
/// without cloning. The caller should handle this case by using the original slice.
/// When query is present, returns Some(filtered_vec).
pub fn filter_items_opt(items: &[FileEntry], query: &str) -> Option<Vec<FileEntry>> {
    filtering::filter_items_opt(items, query)
}

/// Filters items based on a query string. Returns a new Vec of matching items.
///
/// DEPRECATED: Use filter_items_opt() for better performance when query is empty.
/// This function is kept for backwards compatibility.
pub fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry> {
    filtering::filter_items(items, query)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_entry::ends_with_ignore_case;
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
            is_hidden: false,
            recycle_bin: None,
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
            is_hidden: false,
            recycle_bin: None,
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
    fn test_ends_with_ignore_case_unicode_safe() {
        assert!(ends_with_ignore_case(
            "06 Os Cavaleiros do Zodíaco Saga Hades Santuário.zip",
            ".zip"
        ));
        assert!(!ends_with_ignore_case(
            "06 Os Cavaleiros do Zodíaco Saga Hades Santuário",
            ".zip"
        ));
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

    #[test]
    fn test_sort_by_date_recycle_bin() {
        // Test sorting by date in recycle bin — with full recycle metadata,
        // the sort uses the numeric `modified` field (date_deleted_unix)
        // which is more reliable than parsing the localized date string.
        let mut items = vec![
            FileEntry {
                path: PathBuf::from("file1.txt"),
                name: "file1.txt".to_string(),
                is_dir: false,
                size: 100,
                modified: 1706781600, // 2024-02-01 10:00 UTC
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "02/01/2024 10:00".to_string(),
                    original_path: PathBuf::from(r"C:\orig\file1.txt"),
                })),
            },
            FileEntry {
                path: PathBuf::from("file2.txt"),
                name: "file2.txt".to_string(),
                is_dir: false,
                size: 200,
                modified: 1704106200, // 2024-01-01 15:30 UTC
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "01/01/2024 15:30".to_string(),
                    original_path: PathBuf::from(r"C:\orig\file2.txt"),
                })),
            },
            FileEntry {
                path: PathBuf::from("file3.txt"),
                name: "file3.txt".to_string(),
                is_dir: false,
                size: 300,
                modified: 1709283900, // 2024-03-01 08:15 UTC
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "03/01/2024 08:15".to_string(),
                    original_path: PathBuf::from(r"C:\orig\file3.txt"),
                })),
            },
        ];

        // Sort by date (ascending) — uses numeric modified (date_deleted_unix)
        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);

        // file2 deleted first (Jan), then file1 (Feb), then file3 (Mar)
        assert_eq!(items[0].name, "file2.txt");
        assert_eq!(items[1].name, "file1.txt");
        assert_eq!(items[2].name, "file3.txt");
    }

    #[test]
    fn test_sort_by_date_normal_files() {
        // Test sorting by date for normal files (uses modified)
        let mut items = vec![
            create_test_file("file1.txt", 100, 1000), // old modified
            create_test_file("file2.txt", 200, 2000), // recent modified
            create_test_file("file3.txt", 300, 3000), // most recent modified
        ];

        // Sort by date (ascending)
        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);

        // Verify sorted by modification dates
        assert_eq!(items[0].name, "file1.txt");
        assert_eq!(items[1].name, "file2.txt");
        assert_eq!(items[2].name, "file3.txt");
    }

    #[test]
    fn test_sort_by_date_recycle_bin_across_months() {
        let mut items = vec![
            FileEntry {
                path: PathBuf::from("jan.txt"),
                name: "jan.txt".to_string(),
                is_dir: false,
                size: 1,
                modified: 0,
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "12/01/2024 10:00".to_string(),
                    original_path: PathBuf::from(r"C:\orig\jan.txt"),
                })),
            },
            FileEntry {
                path: PathBuf::from("fev.txt"),
                name: "fev.txt".to_string(),
                is_dir: false,
                size: 1,
                modified: 0,
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "02/02/2024 10:00".to_string(),
                    original_path: PathBuf::from(r"C:\orig\fev.txt"),
                })),
            },
        ];

        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);
        assert_eq!(items[0].name, "jan.txt");
        assert_eq!(items[1].name, "fev.txt");
    }
    #[test]
    fn test_sort_by_date_recycle_bin_prefers_numeric_timestamp() {
        // Timestamps intentionally contradict display strings.
        let mut items = vec![
            FileEntry {
                path: PathBuf::from("older.txt"),
                name: "older.txt".to_string(),
                is_dir: false,
                size: 1,
                modified: 1_704_067_200, // 2024-01-03 UTC
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "10/12/2029 10:00".to_string(),
                    original_path: PathBuf::from(r"C:\orig\older.txt"),
                })),
            },
            FileEntry {
                path: PathBuf::from("newer.txt"),
                name: "newer.txt".to_string(),
                is_dir: false,
                size: 1,
                modified: 1_704_153_600, // 2024-01-04 UTC
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                is_hidden: false,
                recycle_bin: Some(Box::new(crate::domain::file_entry::RecycleBinMeta {
                    deletion_date: "01/01/2000 00:00".to_string(),
                    original_path: PathBuf::from(r"C:\orig\newer.txt"),
                })),
            },
        ];

        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);
        assert_eq!(items[0].name, "older.txt");
        assert_eq!(items[1].name, "newer.txt");
    }
}
