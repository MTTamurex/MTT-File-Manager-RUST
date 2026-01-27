use crate::domain::file_entry::{FileEntry, FoldersPosition, SortMode};
use rayon::prelude::*;
use std::cmp::Ordering;

/// PERFORMANCE: Check if string ends with suffix (case-insensitive) without allocation.
#[inline]
fn ends_with_ignore_case(s: &str, suffix: &str) -> bool {
    if s.len() < suffix.len() {
        return false;
    }
    let start = s.len() - suffix.len();
    s[start..].chars()
        .flat_map(|c| c.to_lowercase())
        .eq(suffix.chars().flat_map(|c| c.to_lowercase()))
}

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
    // Helper to check if an item is a "true" directory (not a ZIP file)
    // PERFORMANCE: Uses ends_with_ignore_case to avoid allocation
    let is_true_dir = |item: &FileEntry| -> bool {
        item.is_dir && !ends_with_ignore_case(&item.name, ".zip")
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
        // PERFORMANCE: Uses cmp_ignore_case for zero-allocation comparison
        let ordering = match mode {
            SortMode::Name => natord::compare_ignore_case(&a.name, &b.name),
            SortMode::Date => a.modified.cmp(&b.modified),
            SortMode::Size => a.size.cmp(&b.size),
            SortMode::Type => {
                // PERFORMANCE: Compare extensions without allocation using OsStr
                let ext_a = a.path.extension().map(|e| e.to_ascii_lowercase());
                let ext_b = b.path.extension().map(|e| e.to_ascii_lowercase());
                match ext_a.cmp(&ext_b) {
                    Ordering::Equal => natord::compare_ignore_case(&a.name, &b.name),
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

/// PERFORMANCE: Check if haystack contains needle (case-insensitive) without allocation.
#[inline]
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }

    // Simple sliding window approach
    let needle_lower: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    let haystack_chars: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();

    haystack_chars.windows(needle_lower.len())
        .any(|window| window == needle_lower.as_slice())
}

/// Filters items based on a query string.
///
/// PERFORMANCE: When query is empty, returns None to signal "use all items"
/// without cloning. The caller should handle this case by using the original slice.
/// When query is present, returns Some(filtered_vec).
pub fn filter_items_opt(items: &[FileEntry], query: &str) -> Option<Vec<FileEntry>> {
    if query.is_empty() {
        return None; // Signal: use original items without clone
    }

    // PERFORMANCE: Use case-insensitive contains without repeated allocations
    Some(items
        .iter()
        .filter(|item| contains_ignore_case(&item.name, query))
        .cloned()
        .collect())
}

/// Filters items based on a query string. Returns a new Vec of matching items.
///
/// DEPRECATED: Use filter_items_opt() for better performance when query is empty.
/// This function is kept for backwards compatibility.
pub fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry> {
    filter_items_opt(items, query).unwrap_or_else(|| items.to_vec())
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
