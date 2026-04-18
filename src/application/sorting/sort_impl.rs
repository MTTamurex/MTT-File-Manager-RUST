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
        (Some(a_date), Some(b_date)) => {
            let has_recycle_metadata =
                a.recycle_original_path.is_some() && b.recycle_original_path.is_some();
            if has_recycle_metadata && a.modified > 0 && b.modified > 0 {
                return a.modified.cmp(&b.modified);
            }
            match (
                parse_recycle_date_sort_key(a_date),
                parse_recycle_date_sort_key(b_date),
            ) {
                (Some(a_key), Some(b_key)) => a_key.cmp(&b_key),
                _ => a_date.cmp(b_date),
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.modified.cmp(&b.modified),
    }
}

/// Sorts a slice of FileEntry in place based on the given criteria.
/// Uses Rayon for parallel sorting if the list is large (>5000 items).
///
/// PERFORMANCE: Uses zero-allocation case-insensitive comparisons.
pub(super) fn sort_items(
    items: &mut [FileEntry],
    mode: SortMode,
    descending: bool,
    folders_position: FoldersPosition,
) {
    // Helper to check if an item is a "true" directory (not an archive file).
    let is_true_dir = |item: &FileEntry| -> bool { item.is_dir && !is_archive_extension(&item.name) };

    let compare = |a: &FileEntry, b: &FileEntry| -> Ordering {
        // 1. Folders position logic (ZIP files should be treated as files, not folders).
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

        // 2. Primary sort criteria.
        let ordering = match mode {
            SortMode::Name => natord::compare_ignore_case(&a.name, &b.name),
            SortMode::Date => get_sort_date_for_comparison(a, b),
            SortMode::Size => a.size.cmp(&b.size),
            SortMode::Type => {
                // Extract extension from the cached `name` field (a &str) to avoid
                // OsString allocation from path.extension().to_ascii_lowercase().
                let ext_a = a.name.rsplit_once('.').map(|(_, e)| e);
                let ext_b = b.name.rsplit_once('.').map(|(_, e)| e);
                let ext_ord = match (ext_a, ext_b) {
                    (Some(ea), Some(eb)) => {
                        // Case-insensitive byte-by-byte comparison, zero allocations.
                        ea.as_bytes().iter().map(|b| b.to_ascii_lowercase())
                            .cmp(eb.as_bytes().iter().map(|b| b.to_ascii_lowercase()))
                    }
                    (Some(_), None) => Ordering::Greater,
                    (None, Some(_)) => Ordering::Less,
                    (None, None) => Ordering::Equal,
                };
                match ext_ord {
                    Ordering::Equal => natord::compare_ignore_case(&a.name, &b.name),
                    other => other,
                }
            }
            SortMode::DriveTotalSpace => {
                let total_a = a.drive_info.as_ref().map(|d| d.total_space).unwrap_or(0);
                let total_b = b.drive_info.as_ref().map(|d| d.total_space).unwrap_or(0);
                total_a.cmp(&total_b)
            }
            SortMode::DriveFreeSpace => {
                let free_a = a.drive_info.as_ref().map(|d| d.free_space).unwrap_or(0);
                let free_b = b.drive_info.as_ref().map(|d| d.free_space).unwrap_or(0);
                free_a.cmp(&free_b)
            }
        };

        // 3. Apply descending direction.
        if descending {
            ordering.reverse()
        } else {
            ordering
        }
    };

    const PARALLEL_THRESHOLD: usize = 5000;

    if items.len() > PARALLEL_THRESHOLD {
        items.par_sort_by(compare);
    } else {
        items.sort_by(compare);
    }
}
