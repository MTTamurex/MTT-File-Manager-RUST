use crate::file_index::FileRecord;
use std::collections::{HashMap, HashSet};

/// FRN -> FileRecord store optimized for the service's steady state.
///
/// Full scans and DB loads insert into a mutable overlay HashMap. Once the
/// index stabilizes, `shrink_to_fit` compacts records into two sorted arrays:
/// one for FRNs and one for records. USN deltas continue to use the overlay,
/// and removed base records are tracked by tombstones until the next compaction.
#[derive(Debug)]
pub struct RecordStore {
    base_frns: Vec<u64>,
    base_records: Vec<FileRecord>,
    overlay: HashMap<u64, FileRecord>,
    removed: HashSet<u64>,
    live_len: usize,
}

impl RecordStore {
    pub fn new() -> Self {
        Self {
            base_frns: Vec::new(),
            base_records: Vec::new(),
            overlay: HashMap::new(),
            removed: HashSet::new(),
            live_len: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            base_frns: Vec::new(),
            base_records: Vec::new(),
            overlay: HashMap::with_capacity(capacity),
            removed: HashSet::new(),
            live_len: 0,
        }
    }

    pub fn from_sorted_parts(
        base_frns: Vec<u64>,
        base_records: Vec<FileRecord>,
    ) -> Result<Self, String> {
        if base_frns.len() != base_records.len() {
            return Err("record store parts have different lengths".to_string());
        }
        if base_frns.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("record store FRNs are not strictly sorted".to_string());
        }
        let live_len = base_frns.len();
        Ok(Self {
            base_frns,
            base_records,
            overlay: HashMap::new(),
            removed: HashSet::new(),
            live_len,
        })
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.live_len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.live_len == 0
    }

    #[inline]
    pub fn get(&self, frn: &u64) -> Option<&FileRecord> {
        if let Some(record) = self.overlay.get(frn) {
            return Some(record);
        }

        if self.removed.contains(frn) {
            return None;
        }

        self.base_frns
            .binary_search(frn)
            .ok()
            .map(|index| &self.base_records[index])
    }

    #[inline]
    pub fn get_mut(&mut self, frn: &u64) -> Option<&mut FileRecord> {
        if let Some(record) = self.overlay.get_mut(frn) {
            return Some(record);
        }

        if self.removed.contains(frn) {
            return None;
        }

        let index = self.base_frns.binary_search(frn).ok()?;
        self.base_records.get_mut(index)
    }

    pub fn insert(&mut self, frn: u64, record: FileRecord) -> Option<FileRecord> {
        if let Ok(index) = self.base_frns.binary_search(&frn) {
            let old = if self.removed.remove(&frn) {
                self.live_len += 1;
                None
            } else {
                Some(self.base_records[index])
            };
            self.base_records[index] = record;
            return old;
        }

        let old = self.overlay.insert(frn, record);
        if old.is_none() {
            self.live_len += 1;
        }
        old
    }

    pub fn remove(&mut self, frn: &u64) -> Option<FileRecord> {
        if let Some(record) = self.overlay.remove(frn) {
            self.live_len -= 1;
            return Some(record);
        }

        let index = self.base_frns.binary_search(frn).ok()?;
        if !self.removed.insert(*frn) {
            return None;
        }
        self.live_len -= 1;
        Some(self.base_records[index])
    }

    pub fn clear(&mut self) {
        self.base_frns.clear();
        self.base_records.clear();
        self.overlay.clear();
        self.removed.clear();
        self.live_len = 0;
    }

    pub fn iter(&self) -> RecordIter<'_> {
        RecordIter {
            base_frns: self.base_frns.iter(),
            base_records: self.base_records.iter(),
            removed: &self.removed,
            overlay: self.overlay.iter(),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &u64> + '_ {
        let removed = &self.removed;
        self.base_frns
            .iter()
            .filter(move |frn| !removed.contains(*frn))
            .chain(self.overlay.keys())
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut FileRecord> + '_ {
        let removed = &self.removed;
        self.base_frns
            .iter()
            .zip(self.base_records.iter_mut())
            .filter_map(move |(frn, record)| (!removed.contains(frn)).then_some(record))
            .chain(self.overlay.values_mut())
    }

    pub fn compact(&mut self) {
        if self.live_len == 0 {
            self.clear();
            self.base_frns.shrink_to_fit();
            self.base_records.shrink_to_fit();
            self.overlay.shrink_to_fit();
            self.removed.shrink_to_fit();
            return;
        }

        let mut pairs: Vec<(u64, FileRecord)> =
            if self.base_frns.is_empty() && self.removed.is_empty() {
                self.overlay.drain().collect()
            } else {
                self.iter().map(|(&frn, &record)| (frn, record)).collect()
            };

        pairs.sort_unstable_by_key(|(frn, _)| *frn);
        pairs.dedup_by_key(|(frn, _)| *frn);

        self.base_frns.clear();
        self.base_records.clear();
        self.base_frns.reserve_exact(pairs.len());
        self.base_records.reserve_exact(pairs.len());

        for (frn, record) in pairs {
            self.base_frns.push(frn);
            self.base_records.push(record);
        }

        self.overlay.clear();
        self.removed.clear();
        self.live_len = self.base_frns.len();
    }

    pub fn shrink_to_fit(&mut self) {
        self.compact();
        self.base_frns.shrink_to_fit();
        self.base_records.shrink_to_fit();
        self.overlay.shrink_to_fit();
        self.removed.shrink_to_fit();
    }

    pub fn estimated_heap_bytes(&self) -> usize {
        let base = self
            .base_frns
            .capacity()
            .saturating_mul(std::mem::size_of::<u64>())
            .saturating_add(
                self.base_records
                    .capacity()
                    .saturating_mul(std::mem::size_of::<FileRecord>()),
            );
        let overlay = self
            .overlay
            .capacity()
            .saturating_mul(std::mem::size_of::<u64>() + std::mem::size_of::<FileRecord>() + 1);
        let removed = self
            .removed
            .capacity()
            .saturating_mul(std::mem::size_of::<u64>() + 1);
        base.saturating_add(overlay).saturating_add(removed)
    }
}

impl Default for RecordStore {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> IntoIterator for &'a RecordStore {
    type Item = (&'a u64, &'a FileRecord);
    type IntoIter = RecordIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct RecordIter<'a> {
    base_frns: std::slice::Iter<'a, u64>,
    base_records: std::slice::Iter<'a, FileRecord>,
    removed: &'a HashSet<u64>,
    overlay: std::collections::hash_map::Iter<'a, u64, FileRecord>,
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = (&'a u64, &'a FileRecord);

    fn next(&mut self) -> Option<Self::Item> {
        for frn in self.base_frns.by_ref() {
            let record = self
                .base_records
                .next()
                .expect("record store base arrays must stay aligned");
            if !self.removed.contains(frn) {
                return Some((frn, record));
            }
        }
        self.overlay.next()
    }
}

#[cfg(test)]
mod tests {
    use super::RecordStore;
    use crate::file_index::FileRecord;

    fn record(parent_ref: u64, size: u64) -> FileRecord {
        FileRecord {
            parent_ref,
            size,
            name_offset: 0,
            name_len: 0,
            is_dir: false,
            _pad: 0,
        }
    }

    #[test]
    fn compact_preserves_lookup_mutation_and_iteration() {
        let mut store = RecordStore::with_capacity(4);
        store.insert(20, record(5, 2));
        store.insert(10, record(5, 1));
        store.shrink_to_fit();

        store.get_mut(&10).unwrap().size = 11;
        store.insert(30, record(10, 3));
        assert_eq!(store.remove(&20).unwrap().size, 2);

        let pairs = store
            .iter()
            .map(|(&frn, rec)| (frn, rec.size))
            .collect::<Vec<_>>();
        assert_eq!(pairs, vec![(10, 11), (30, 3)]);

        store.shrink_to_fit();
        assert_eq!(store.get(&10).unwrap().size, 11);
        assert!(store.get(&20).is_none());
        assert_eq!(store.len(), 2);
    }
}
