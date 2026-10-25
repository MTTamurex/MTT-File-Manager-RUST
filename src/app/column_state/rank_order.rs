//! Apply dense ranks linearly; use cached keys for sparse filtered subsets.
use crate::domain::file_entry::FileEntry;
use std::{collections::HashMap, path::PathBuf};

pub(super) fn apply(items: &mut [FileEntry], ranks: &HashMap<PathBuf, usize>) {
    if items.len() < 2 {
        return;
    }
    let missing = ranks.len();
    if missing > items.len().saturating_mul(4) {
        items.sort_by_cached_key(|item| {
            ranks
                .get(&item.path)
                .copied()
                .unwrap_or(missing)
                .min(missing)
        });
        return;
    }
    let mut destinations: Vec<_> = items
        .iter()
        .map(|item| {
            ranks
                .get(&item.path)
                .copied()
                .unwrap_or(missing)
                .min(missing)
        })
        .collect();
    let mut offsets = vec![0; missing + 1];
    for &rank in &destinations {
        offsets[rank] += 1;
    }
    let mut position = 0;
    for count in &mut offsets {
        let length = *count;
        *count = position;
        position += length;
    }
    for destination in &mut destinations {
        let rank = *destination;
        *destination = offsets[rank];
        offsets[rank] += 1;
    }
    for index in 0..items.len() {
        while destinations[index] != index {
            let target = destinations[index];
            items.swap(index, target);
            destinations.swap(index, target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_match_stable_reference_sort_for_full_and_filtered_lists() {
        let source: Vec<_> = (0..64)
            .map(|index| FileEntry::from_path(format!("file{index}.txt").into(), false))
            .collect();
        let ranks: HashMap<_, _> = source
            .iter()
            .rev()
            .enumerate()
            .map(|(rank, item)| (item.path.clone(), rank))
            .collect();
        for step in [1, 2, 3, 8] {
            let mut items: Vec<_> = source.iter().step_by(step).cloned().collect();
            let mut expected = items.clone();
            expected.sort_by_key(|item| ranks[&item.path]);
            apply(&mut items, &ranks);
            assert_eq!(
                items.iter().map(|item| &item.path).collect::<Vec<_>>(),
                expected.iter().map(|item| &item.path).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn duplicates_and_unknown_items_keep_their_relative_order() {
        let ranks = HashMap::from([("file1.txt".into(), 0), ("file2.txt".into(), 1)]);
        let mut items: Vec<_> = [
            "file2.txt",
            "missing1.txt",
            "file1.txt",
            "missing2.txt",
            "file1.txt",
        ]
        .into_iter()
        .map(|path| FileEntry::from_path(path.into(), false))
        .collect();
        for (index, item) in items.iter_mut().enumerate() {
            item.size = index as u64;
        }
        apply(&mut items, &ranks);
        assert_eq!(
            items.iter().map(|item| item.size).collect::<Vec<_>>(),
            vec![2, 4, 0, 1, 3]
        );
        apply(&mut [], &ranks);
        let order: Vec<_> = items.iter().map(|item| item.size).collect();
        apply(&mut items, &HashMap::new());
        assert_eq!(
            items.iter().map(|item| item.size).collect::<Vec<_>>(),
            order
        );
    }
}
