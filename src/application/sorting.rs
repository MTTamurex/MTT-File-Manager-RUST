use crate::domain::file_entry::{is_archive_extension, FileEntry, FoldersPosition, SortMode};
use rayon::prelude::*;
use std::cmp::Ordering;

/// Parse recycle bin dates like `dd/mm/yyyy hh:mm` (or with seconds) into
/// sortable tuple `(year, month, day, hour, minute, second)`.
#[inline]
fn parse_recycle_date_sort_key(date: &str) -> Option<(u32, u32, u32, u32, u32, u32)> {
    let trimmed = date.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut parts = trimmed.split_whitespace();
    let date_part = parts.next()?;
    let time_part = parts.next().unwrap_or("00:00:00");

    let mut date_it = date_part.split(['/', '-', '.']);
    let day = date_it.next()?.parse::<u32>().ok()?;
    let month = date_it.next()?.parse::<u32>().ok()?;
    let year = date_it.next()?.parse::<u32>().ok()?;
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) || year < 1601 {
        return None;
    }

    let mut time_it = time_part.split(':');
    let hour = time_it
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let minute = time_it
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    let second = time_it
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    Some((year, month, day, hour, minute, second))
}

/// Helper function to get the appropriate date for sorting.
/// For recycle bin items with deletion_date, uses the deletion date string for comparison.
/// For all other items, uses the modified timestamp.
///
/// For recycle bin items, attempts semantic parsing of date string first, and
/// falls back to lexicographic comparison only when parsing fails.
fn get_sort_date_for_comparison(a: &FileEntry, b: &FileEntry) -> Ordering {
    match (&a.deletion_date, &b.deletion_date) {
        // Ambos têm data de exclusão (lixeira): compara strings diretamente
        (Some(a_date), Some(b_date)) => match (
            parse_recycle_date_sort_key(a_date),
            parse_recycle_date_sort_key(b_date),
        ) {
            (Some(a_key), Some(b_key)) => a_key.cmp(&b_key),
            _ => a_date.cmp(b_date),
        },
        // Apenas um tem data de exclusão: considera que items da lixeira vêm primeiro
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        // Nenhum tem data de exclusão: usa modified como antes
        (None, None) => a.modified.cmp(&b.modified),
    }
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
    // Helper to check if an item is a "true" directory (not an archive file)
    let is_true_dir =
        |item: &FileEntry| -> bool { item.is_dir && !is_archive_extension(&item.name) };

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
            } else if folders_come_first {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }

        // 2. Primary sort criteria
        // PERFORMANCE: Uses cmp_ignore_case for zero-allocation comparison
        let ordering = match mode {
            SortMode::Name => natord::compare_ignore_case(&a.name, &b.name),
            SortMode::Date => get_sort_date_for_comparison(a, b),
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
            SortMode::DriveTotalSpace => {
                // Ordena por espaço total do drive (do maior para o menor por padrão)
                let total_a = a.drive_info.as_ref().map(|d| d.total_space).unwrap_or(0);
                let total_b = b.drive_info.as_ref().map(|d| d.total_space).unwrap_or(0);
                total_a.cmp(&total_b)
            }
            SortMode::DriveFreeSpace => {
                // Ordena por espaço livre do drive (do maior para o menor por padrão)
                let free_a = a.drive_info.as_ref().map(|d| d.free_space).unwrap_or(0);
                let free_b = b.drive_info.as_ref().map(|d| d.free_space).unwrap_or(0);
                free_a.cmp(&free_b)
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

/// PERFORMANCE: Check if haystack contains needle (case-insensitive) using precomputed needle.
/// For ASCII strings, uses fast byte-by-byte comparison without allocation.
/// Falls back to Unicode-aware comparison for non-ASCII strings.
#[inline]
fn contains_ignore_case_precomputed(
    haystack: &str,
    needle_lower: &[char],
    needle_ascii_lower: Option<&[u8]>,
) -> bool {
    if needle_lower.is_empty() {
        return true;
    }

    // PERFORMANCE: Fast path for ASCII strings (majority of filenames)
    // Uses byte-by-byte comparison without any allocation
    if haystack.is_ascii() {
        if let Some(needle_bytes) = needle_ascii_lower {
            return haystack.as_bytes().windows(needle_bytes.len()).any(|w| {
                w.iter()
                    .zip(needle_bytes.iter())
                    .all(|(h, n)| h.to_ascii_lowercase() == *n)
            });
        }
    }

    // Fallback: Unicode-aware comparison using Vec<char>
    let haystack_chars: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    haystack_chars
        .windows(needle_lower.len())
        .any(|window| window == needle_lower)
}

/// Filters items based on a query string.
///
/// PERFORMANCE: When query is empty, returns None to signal "use all items"
/// without cloning. The caller should handle this case by using the original slice.
/// When query is present, returns Some(filtered_vec).
///
/// PERFORMANCE: Precomputes needle_lower once before the filter loop to avoid
/// repeated allocations in contains_ignore_case.
pub fn filter_items_opt(items: &[FileEntry], query: &str) -> Option<Vec<FileEntry>> {
    if query.is_empty() {
        return None; // Signal: use original items without clone
    }

    // PERFORMANCE: Precompute needle_lower once for the entire filter operation
    let needle_lower: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let needle_ascii_lower: Option<Vec<u8>> = if needle_lower.iter().all(|c| c.is_ascii()) {
        Some(needle_lower.iter().map(|c| *c as u8).collect())
    } else {
        None
    };

    Some(
        items
            .iter()
            .filter(|item| {
                contains_ignore_case_precomputed(
                    &item.name,
                    &needle_lower,
                    needle_ascii_lower.as_deref(),
                )
            })
            .cloned()
            .collect(),
    )
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
        // Testa ordenação por data na lixeira (usa deletion_date)
        let mut items = vec![
            FileEntry {
                path: PathBuf::from("file1.txt"),
                name: "file1.txt".to_string(),
                is_dir: false,
                size: 100,
                modified: 1000, // data antiga
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                deletion_date: Some("02/01/2024 10:00".to_string()),
                recycle_original_path: None,
            },
            FileEntry {
                path: PathBuf::from("file2.txt"),
                name: "file2.txt".to_string(),
                is_dir: false,
                size: 200,
                modified: 2000, // data mais recente
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                deletion_date: Some("01/01/2024 15:30".to_string()),
                recycle_original_path: None,
            },
            FileEntry {
                path: PathBuf::from("file3.txt"),
                name: "file3.txt".to_string(),
                is_dir: false,
                size: 300,
                modified: 3000, // data mais recente ainda
                folder_cover: None,
                drive_info: None,
                sync_status: crate::domain::file_entry::SyncStatus::None,
                deletion_date: Some("03/01/2024 08:15".to_string()),
                recycle_original_path: None,
            },
        ];

        // Ordena por data (ascendente)
        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);

        // Verifica se está ordenado pelas datas de exclusão (não por modified)
        // file2 foi excluído primeiro (01/01), depois file1 (02/01), depois file3 (03/01)
        assert_eq!(items[0].name, "file2.txt");
        assert_eq!(items[1].name, "file1.txt");
        assert_eq!(items[2].name, "file3.txt");
    }

    #[test]
    fn test_sort_by_date_normal_files() {
        // Testa ordenação por data para arquivos normais (usa modified)
        let mut items = vec![
            create_test_file("file1.txt", 100, 1000), // modified antigo
            create_test_file("file2.txt", 200, 2000), // modified recente
            create_test_file("file3.txt", 300, 3000), // modified mais recente
        ];

        // Ordena por data (ascendente)
        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);

        // Verifica se está ordenado pelas datas de modificação
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
                deletion_date: Some("12/01/2024 10:00".to_string()),
                recycle_original_path: None,
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
                deletion_date: Some("02/02/2024 10:00".to_string()),
                recycle_original_path: None,
            },
        ];

        sort_items(&mut items, SortMode::Date, false, FoldersPosition::Mixed);
        assert_eq!(items[0].name, "jan.txt");
        assert_eq!(items[1].name, "fev.txt");
    }
}
