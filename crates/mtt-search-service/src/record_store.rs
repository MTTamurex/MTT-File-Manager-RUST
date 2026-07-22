use crate::file_index::FileRecord;
use memmap2::Mmap;
use std::collections::{HashMap, HashSet};

/// On-disk / in-memory pairing of an FRN with its [`FileRecord`].
///
/// Laid out so a memory-mapped records region can be reinterpreted as
/// `&[RecordEntry]` with zero copies. On little-endian targets (the only ones
/// this Windows service runs on) the byte layout equals `frn.to_le_bytes()`
/// followed by the raw `FileRecord` bytes, matching what [`crate::index_db::binary`]
/// writes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RecordEntry {
    pub frn: u64,
    pub rec: FileRecord,
}

// Compile-time layout guarantees for the zero-copy mmap cast.
const _: () = {
    assert!(std::mem::size_of::<RecordEntry>() == 32);
    assert!(std::mem::align_of::<RecordEntry>() == 8);
    assert!(std::mem::offset_of!(RecordEntry, frn) == 0);
    assert!(std::mem::offset_of!(RecordEntry, rec) == 8);
};

/// Immutable base of the record store.
///
/// `Owned` holds two parallel sorted arrays (built by scans and DB loads).
/// `Mapped` borrows a read-only, file-backed memory mapping of the binary
/// index's records region — record bytes live in evictable OS page cache
/// instead of private heap. Because a mapping cannot be mutated in place,
/// updates to a mapped record are copied into the store's overlay (see
/// [`RecordStore`]).
enum RecordBase {
    Owned {
        frns: Vec<u64>,
        records: Vec<FileRecord>,
    },
    Mapped {
        mmap: Mmap,
        count: usize,
    },
}

impl std::fmt::Debug for RecordBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordBase::Owned { frns, .. } => {
                f.debug_struct("Owned").field("len", &frns.len()).finish()
            }
            RecordBase::Mapped { count, .. } => {
                f.debug_struct("Mapped").field("count", count).finish()
            }
        }
    }
}

impl RecordBase {
    #[inline]
    fn entries(mmap: &Mmap, count: usize) -> &[RecordEntry] {
        // SAFETY: `RecordStore::from_mmap` verified alignment and exact size.
        // RecordEntry has no padding and contains only integer fields, so every
        // possible byte pattern is valid even if the backing file changes.
        let ptr = mmap.as_ptr() as *const RecordEntry;
        unsafe { std::slice::from_raw_parts(ptr, count) }
    }

    #[inline]
    fn len(&self) -> usize {
        match self {
            RecordBase::Owned { frns, .. } => frns.len(),
            RecordBase::Mapped { count, .. } => *count,
        }
    }

    /// Binary-search the sorted base for `frn`, returning its index.
    #[inline]
    fn search(&self, frn: u64) -> Option<usize> {
        match self {
            RecordBase::Owned { frns, .. } => frns.binary_search(&frn).ok(),
            RecordBase::Mapped { mmap, count } => Self::entries(mmap, *count)
                .binary_search_by(|entry| entry.frn.cmp(&frn))
                .ok(),
        }
    }

    #[inline]
    fn frn_ref_at(&self, index: usize) -> &u64 {
        match self {
            RecordBase::Owned { frns, .. } => &frns[index],
            RecordBase::Mapped { mmap, count } => &Self::entries(mmap, *count)[index].frn,
        }
    }

    #[inline]
    fn rec_ref_at(&self, index: usize) -> &FileRecord {
        match self {
            RecordBase::Owned { records, .. } => &records[index],
            RecordBase::Mapped { mmap, count } => &Self::entries(mmap, *count)[index].rec,
        }
    }

    #[inline]
    fn rec_at(&self, index: usize) -> FileRecord {
        *self.rec_ref_at(index)
    }

    #[inline]
    fn is_mapped(&self) -> bool {
        matches!(self, RecordBase::Mapped { .. })
    }
}

/// FRN -> FileRecord store optimized for the service's steady state.
///
/// Full scans and DB loads insert into a mutable overlay HashMap over an
/// `Owned` base. Once the index stabilizes, `shrink_to_fit`/`compact` compact
/// records into two sorted arrays. Binary-cache loads may instead use a
/// `Mapped` base backed by the on-disk index file.
///
/// The `overlay` is authoritative for any FRN it contains. `removed` lists base
/// FRNs whose base slot must be ignored — either because the record was deleted
/// or because a mapped base record was copied into the overlay on write (copy
/// on write). `overlay` and `base \ removed` are therefore always disjoint sets
/// of live FRNs, keeping `live_len` a simple running count.
#[derive(Debug)]
pub struct RecordStore {
    base: RecordBase,
    overlay: HashMap<u64, FileRecord>,
    removed: HashSet<u64>,
    live_len: usize,
}

impl RecordStore {
    pub fn new() -> Self {
        Self {
            base: RecordBase::Owned {
                frns: Vec::new(),
                records: Vec::new(),
            },
            overlay: HashMap::new(),
            removed: HashSet::new(),
            live_len: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            base: RecordBase::Owned {
                frns: Vec::new(),
                records: Vec::new(),
            },
            overlay: HashMap::with_capacity(capacity),
            removed: HashSet::new(),
            live_len: 0,
        }
    }

    /// Create a store optimized for bulk input that arrives in strictly
    /// increasing FRN order, such as a raw sequential MFT scan.
    pub fn with_sorted_capacity(capacity: usize) -> Self {
        Self {
            base: RecordBase::Owned {
                frns: Vec::with_capacity(capacity),
                records: Vec::with_capacity(capacity),
            },
            overlay: HashMap::new(),
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
            base: RecordBase::Owned {
                frns: base_frns,
                records: base_records,
            },
            overlay: HashMap::new(),
            removed: HashSet::new(),
            live_len,
        })
    }

    /// Build a store whose base is a read-only memory mapping of the records
    /// region of a binary index. `count` is the number of `RecordEntry` records
    /// in the mapping. The writer stores records sorted by FRN and the file is
    /// HMAC-authenticated on load, so the base is trusted to be strictly sorted
    /// without a scan (which would defeat lazy paging).
    pub fn from_mmap(mmap: Mmap, count: usize) -> Result<Self, String> {
        let expected = count
            .checked_mul(std::mem::size_of::<RecordEntry>())
            .ok_or_else(|| "record mmap size overflow".to_string())?;
        if mmap.len() != expected {
            return Err(format!(
                "record mmap size mismatch: expected {} got {}",
                expected,
                mmap.len()
            ));
        }
        if !(mmap.as_ptr() as usize).is_multiple_of(std::mem::align_of::<RecordEntry>()) {
            return Err("record mmap is not 8-byte aligned".to_string());
        }
        Ok(Self {
            base: RecordBase::Mapped { mmap, count },
            overlay: HashMap::new(),
            removed: HashSet::new(),
            live_len: count,
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

    /// Whether the base is a file-backed memory mapping.
    #[inline]
    pub fn is_mapped(&self) -> bool {
        self.base.is_mapped()
    }

    #[inline]
    pub fn get(&self, frn: &u64) -> Option<&FileRecord> {
        if let Some(record) = self.overlay.get(frn) {
            return Some(record);
        }
        if self.removed.contains(frn) {
            return None;
        }
        let index = self.base.search(*frn)?;
        Some(self.base.rec_ref_at(index))
    }

    #[inline]
    pub fn get_mut(&mut self, frn: &u64) -> Option<&mut FileRecord> {
        if self.overlay.contains_key(frn) {
            return self.overlay.get_mut(frn);
        }
        if self.removed.contains(frn) {
            return None;
        }
        let index = self.base.search(*frn)?;
        if self.base.is_mapped() {
            // Mapped base is read-only: copy the record into the overlay and
            // shadow the base slot so future lookups resolve through the overlay.
            let record = self.base.rec_at(index);
            self.removed.insert(*frn);
            self.overlay.insert(*frn, record);
            return self.overlay.get_mut(frn);
        }
        // Owned base: mutate in place.
        if let RecordBase::Owned { records, .. } = &mut self.base {
            records.get_mut(index)
        } else {
            unreachable!("base is owned: checked !is_mapped above")
        }
    }

    pub fn insert(&mut self, frn: u64, record: FileRecord) -> Option<FileRecord> {
        // Overlay already authoritative for this FRN (overlay-only or shadowed).
        if let Some(slot) = self.overlay.get_mut(&frn) {
            let old = *slot;
            *slot = record;
            return Some(old);
        }

        if let Some(index) = self.base.search(frn) {
            if let RecordBase::Owned { records, .. } = &mut self.base {
                let old = if self.removed.remove(&frn) {
                    self.live_len += 1;
                    None
                } else {
                    Some(records[index])
                };
                records[index] = record;
                return old;
            }

            // Mapped base: cannot mutate in place.
            if self.removed.contains(&frn) {
                // Previously deleted base slot — revive as a new overlay entry.
                // The FRN stays in `removed` so the stale base slot is ignored.
                self.overlay.insert(frn, record);
                self.live_len += 1;
                return None;
            }
            // Live base record — copy on write: shadow base slot + store overlay.
            let old = self.base.rec_at(index);
            self.removed.insert(frn);
            self.overlay.insert(frn, record);
            return Some(old);
        }

        // Brand-new FRN, not present in base or overlay.
        self.overlay.insert(frn, record);
        self.live_len += 1;
        None
    }

    /// Append a new record when the caller can guarantee increasing FRNs.
    /// Returns the record back to the caller if the fast path cannot be used.
    pub fn push_sorted(&mut self, frn: u64, record: FileRecord) -> Result<(), FileRecord> {
        if !self.overlay.is_empty() || !self.removed.is_empty() {
            return Err(record);
        }
        match &mut self.base {
            RecordBase::Owned { frns, records } => {
                if frns.last().is_some_and(|last| *last >= frn) {
                    return Err(record);
                }
                frns.push(frn);
                records.push(record);
                self.live_len += 1;
                Ok(())
            }
            RecordBase::Mapped { .. } => Err(record),
        }
    }

    pub fn remove(&mut self, frn: &u64) -> Option<FileRecord> {
        if let Some(record) = self.overlay.remove(frn) {
            self.live_len -= 1;
            // If this overlay entry was shadowing a base slot it is already in
            // `removed`, so the base stays ignored — nothing more to do.
            return Some(record);
        }

        let index = self.base.search(*frn)?;
        if !self.removed.insert(*frn) {
            return None;
        }
        self.live_len -= 1;
        Some(self.base.rec_at(index))
    }

    pub fn clear(&mut self) {
        self.base = RecordBase::Owned {
            frns: Vec::new(),
            records: Vec::new(),
        };
        self.overlay.clear();
        self.removed.clear();
        self.live_len = 0;
    }

    pub fn iter(&self) -> RecordIter<'_> {
        RecordIter {
            store: self,
            base_index: 0,
            base_len: self.base.len(),
            overlay: self.overlay.iter(),
        }
    }

    /// Iterate all live records in ascending FRN order. The immutable base is
    /// already sorted, so only overlay references need temporary sorting.
    pub fn iter_sorted(&self) -> SortedRecordIter<'_> {
        let mut overlay: Vec<(&u64, &FileRecord)> = self.overlay.iter().collect();
        overlay.sort_unstable_by_key(|(frn, _)| **frn);
        SortedRecordIter {
            store: self,
            base_index: 0,
            base_len: self.base.len(),
            overlay,
            overlay_index: 0,
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &u64> + '_ {
        self.iter().map(|(frn, _)| frn)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut FileRecord> + '_ {
        // A mapped base is read-only; materialize it so callers can mutate every
        // live record. Only `compact_arena` uses this, and only on owned indices
        // in practice, so the materialization is defensive.
        self.materialize_owned();
        let RecordBase::Owned { frns, records } = &mut self.base else {
            unreachable!("materialize_owned guarantees an Owned base");
        };
        let removed = &self.removed;
        frns.iter()
            .zip(records.iter_mut())
            .filter_map(move |(frn, record)| (!removed.contains(frn)).then_some(record))
            .chain(self.overlay.values_mut())
    }

    /// Convert a mapped base into an owned base holding all live records, sorted
    /// by FRN, clearing the overlay/tombstones. No-op for an already-owned base.
    fn materialize_owned(&mut self) {
        if !self.base.is_mapped() {
            return;
        }
        let mut frns = Vec::with_capacity(self.live_len);
        let mut records = Vec::with_capacity(self.live_len);
        for (&frn, &record) in self.iter_sorted() {
            frns.push(frn);
            records.push(record);
        }
        self.live_len = frns.len();
        self.base = RecordBase::Owned { frns, records };
        self.overlay.clear();
        self.removed.clear();
    }

    /// Prefer contiguous owned arrays when an operation is about to mutate a
    /// large fraction of a mapped base. This avoids a HashMap plus tombstone
    /// entry for nearly every record during bulk size refreshes.
    pub fn prepare_bulk_mutation(&mut self, mutation_count: usize) {
        if self.base.is_mapped() && mutation_count.saturating_mul(4) >= self.live_len.max(1) {
            self.materialize_owned();
        }
    }

    pub fn compact(&mut self) {
        if self.live_len == 0 {
            self.clear();
            if let RecordBase::Owned { frns, records } = &mut self.base {
                frns.shrink_to_fit();
                records.shrink_to_fit();
            }
            self.overlay.shrink_to_fit();
            self.removed.shrink_to_fit();
            return;
        }

        // A mapped base is file-backed (cheap RAM) and immutable; folding the
        // overlay back would require materializing it into private heap, which
        // defeats the mapping. Deltas are compacted into a fresh base by the
        // next binary save + remap or DB reload, so keep them here.
        if self.base.is_mapped() {
            return;
        }

        if self.overlay.is_empty() && self.removed.is_empty() && self.live_len == self.base.len() {
            return;
        }

        let mut pairs: Vec<(u64, FileRecord)> =
            self.iter().map(|(&frn, &record)| (frn, record)).collect();
        pairs.sort_unstable_by_key(|(frn, _)| *frn);
        pairs.dedup_by_key(|(frn, _)| *frn);

        let mut frns = Vec::with_capacity(pairs.len());
        let mut records = Vec::with_capacity(pairs.len());
        for (frn, record) in pairs {
            frns.push(frn);
            records.push(record);
        }
        self.live_len = frns.len();
        self.base = RecordBase::Owned { frns, records };
        self.overlay.clear();
        self.removed.clear();
    }

    pub fn shrink_to_fit(&mut self) {
        self.compact();
        if let RecordBase::Owned { frns, records } = &mut self.base {
            frns.shrink_to_fit();
            records.shrink_to_fit();
        }
        self.overlay.shrink_to_fit();
        self.removed.shrink_to_fit();
    }

    /// Return sorted compact storage when the base is owned and there are no
    /// overlay/tombstone deltas. A mapped base stores records interleaved
    /// (array-of-structs) and has no separate FRN/record slices, so it returns
    /// `None` and callers fall back to iterating.
    pub fn compact_sorted_slices(&self) -> Option<(&[u64], &[FileRecord])> {
        match &self.base {
            RecordBase::Owned { frns, records } => {
                if self.overlay.is_empty()
                    && self.removed.is_empty()
                    && self.live_len == frns.len()
                    && frns.len() == records.len()
                {
                    Some((frns, records))
                } else {
                    None
                }
            }
            RecordBase::Mapped { .. } => None,
        }
    }

    pub fn estimated_heap_bytes(&self) -> usize {
        // A mapped base lives in file-backed OS page cache, not private heap.
        let base = match &self.base {
            RecordBase::Owned { frns, records } => frns
                .capacity()
                .saturating_mul(std::mem::size_of::<u64>())
                .saturating_add(
                    records
                        .capacity()
                        .saturating_mul(std::mem::size_of::<FileRecord>()),
                ),
            RecordBase::Mapped { .. } => 0,
        };
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
    store: &'a RecordStore,
    base_index: usize,
    base_len: usize,
    overlay: std::collections::hash_map::Iter<'a, u64, FileRecord>,
}

pub struct SortedRecordIter<'a> {
    store: &'a RecordStore,
    base_index: usize,
    base_len: usize,
    overlay: Vec<(&'a u64, &'a FileRecord)>,
    overlay_index: usize,
}

impl<'a> Iterator for RecordIter<'a> {
    type Item = (&'a u64, &'a FileRecord);

    fn next(&mut self) -> Option<Self::Item> {
        while self.base_index < self.base_len {
            let index = self.base_index;
            self.base_index += 1;
            let frn = self.store.base.frn_ref_at(index);
            if !self.store.removed.contains(frn) {
                return Some((frn, self.store.base.rec_ref_at(index)));
            }
        }
        self.overlay.next()
    }
}

impl<'a> Iterator for SortedRecordIter<'a> {
    type Item = (&'a u64, &'a FileRecord);

    fn next(&mut self) -> Option<Self::Item> {
        while self.base_index < self.base_len {
            let frn = self.store.base.frn_ref_at(self.base_index);
            if !self.store.removed.contains(frn) {
                break;
            }
            self.base_index += 1;
        }

        let base = (self.base_index < self.base_len).then(|| {
            (
                self.store.base.frn_ref_at(self.base_index),
                self.store.base.rec_ref_at(self.base_index),
            )
        });
        let overlay = self.overlay.get(self.overlay_index).copied();

        match (base, overlay) {
            (Some(base), Some(overlay)) => match base.0.cmp(overlay.0) {
                std::cmp::Ordering::Less => {
                    self.base_index += 1;
                    Some(base)
                }
                std::cmp::Ordering::Greater => {
                    self.overlay_index += 1;
                    Some(overlay)
                }
                std::cmp::Ordering::Equal => {
                    // Defensive fallback: overlays are authoritative if an
                    // invariant violation leaves the base slot untombstoned.
                    self.base_index += 1;
                    self.overlay_index += 1;
                    Some(overlay)
                }
            },
            (Some(base), None) => {
                self.base_index += 1;
                Some(base)
            }
            (None, Some(overlay)) => {
                self.overlay_index += 1;
                Some(overlay)
            }
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RecordEntry, RecordStore};
    use crate::file_index::FileRecord;
    use memmap2::MmapOptions;
    use std::io::Write;

    fn record(parent_ref: u64, size: u64) -> FileRecord {
        FileRecord {
            parent_ref,
            size,
            name_offset: 0,
            name_len: 0,
            is_dir: 0,
            _pad: 0,
        }
    }

    fn mapped_store(entries: &[(u64, FileRecord)]) -> RecordStore {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mtt-record-store-test-{}-{}.bin",
            std::process::id(),
            suffix
        ));
        {
            let mut file = std::fs::File::create(&path).unwrap();
            for (frn, rec) in entries {
                let entry = RecordEntry {
                    frn: *frn,
                    rec: *rec,
                };
                let bytes: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        &entry as *const RecordEntry as *const u8,
                        std::mem::size_of::<RecordEntry>(),
                    )
                };
                file.write_all(bytes).unwrap();
            }
            file.sync_all().unwrap();
        }
        let file = std::fs::File::open(&path).unwrap();
        let mmap = unsafe { MmapOptions::new().map(&file).unwrap() };
        let store = RecordStore::from_mmap(mmap, entries.len()).unwrap();
        let _ = std::fs::remove_file(&path);
        store
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

    #[test]
    fn mapped_base_reads_are_zero_copy_and_sorted() {
        let store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2)), (30, record(10, 3))]);
        assert!(store.is_mapped());
        assert_eq!(store.len(), 3);
        assert_eq!(store.get(&20).unwrap().size, 2);
        assert!(store.get(&99).is_none());

        let pairs = store
            .iter()
            .map(|(&frn, rec)| (frn, rec.size))
            .collect::<Vec<_>>();
        assert_eq!(pairs, vec![(10, 1), (20, 2), (30, 3)]);
    }

    #[test]
    fn mapped_get_mut_copies_on_write_without_touching_base_neighbors() {
        let mut store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2))]);
        store.get_mut(&10).unwrap().size = 111;
        assert_eq!(store.get(&10).unwrap().size, 111);
        // Untouched record still served from the mapping.
        assert_eq!(store.get(&20).unwrap().size, 2);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn mapped_sorted_iteration_is_stable_across_copy_on_write() {
        let mut store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2)), (30, record(5, 3))]);
        store.get_mut(&10).unwrap().size = 11;
        store.insert(25, record(5, 25));

        let pairs = store
            .iter_sorted()
            .map(|(&frn, rec)| (frn, rec.size))
            .collect::<Vec<_>>();
        assert_eq!(pairs, vec![(10, 11), (20, 2), (25, 25), (30, 3)]);
    }

    #[test]
    fn mapped_insert_overwrite_and_new_and_remove() {
        let mut store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2))]);

        // Overwrite an existing (live) base record -> returns old, len unchanged.
        assert_eq!(store.insert(20, record(5, 22)).unwrap().size, 2);
        assert_eq!(store.get(&20).unwrap().size, 22);
        assert_eq!(store.len(), 2);

        // Brand-new record -> len grows.
        assert!(store.insert(30, record(10, 3)).is_none());
        assert_eq!(store.len(), 3);

        // Remove a base record (tombstone) and an overlay record.
        assert_eq!(store.remove(&10).unwrap().size, 1);
        assert!(store.get(&10).is_none());
        assert_eq!(store.len(), 2);

        // Re-inserting a previously removed base FRN revives it.
        assert!(store.insert(10, record(5, 100)).is_none());
        assert_eq!(store.get(&10).unwrap().size, 100);
        assert_eq!(store.len(), 3);

        let mut pairs = store
            .iter()
            .map(|(&frn, rec)| (frn, rec.size))
            .collect::<Vec<_>>();
        pairs.sort_unstable();
        assert_eq!(pairs, vec![(10, 100), (20, 22), (30, 3)]);
    }

    #[test]
    fn mapped_shrink_to_fit_keeps_base_mapped_with_small_overlay() {
        let mut store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2))]);
        store.get_mut(&10).unwrap().size = 7; // forces a copy-on-write entry
        store.shrink_to_fit();
        // compact() must NOT materialize the mapping.
        assert!(store.is_mapped());
        assert_eq!(store.get(&10).unwrap().size, 7);
        assert_eq!(store.get(&20).unwrap().size, 2);
        assert!(store.compact_sorted_slices().is_none());
    }

    #[test]
    fn mapped_values_mut_materializes_to_owned() {
        let mut store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2))]);
        for rec in store.values_mut() {
            rec.size += 10;
        }
        assert!(!store.is_mapped());
        assert_eq!(store.get(&10).unwrap().size, 11);
        assert_eq!(store.get(&20).unwrap().size, 12);
    }

    #[test]
    fn bulk_materialization_preserves_mixed_mapped_deltas() {
        let mut store = mapped_store(&[(10, record(5, 1)), (20, record(5, 2)), (30, record(5, 3))]);
        store.get_mut(&20).unwrap().size = 22;
        assert_eq!(store.remove(&10).unwrap().size, 1);
        store.insert(10, record(5, 11));
        store.insert(25, record(5, 25));
        store.remove(&30);

        store.prepare_bulk_mutation(4);

        assert!(!store.is_mapped());
        let pairs = store
            .iter_sorted()
            .map(|(&frn, rec)| (frn, rec.size))
            .collect::<Vec<_>>();
        assert_eq!(pairs, vec![(10, 11), (20, 22), (25, 25)]);
    }
}
