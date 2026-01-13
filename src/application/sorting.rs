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
    let compare = |a: &FileEntry, b: &FileEntry| -> Ordering {
        // 1. Folders Position logic
        if folders_position != FoldersPosition::Mixed && a.is_dir != b.is_dir {
            let folders_come_first = folders_position == FoldersPosition::First;
            return if a.is_dir {
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
