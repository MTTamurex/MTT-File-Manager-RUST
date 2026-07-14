use std::collections::{HashMap, HashSet};
use std::time::Duration;

use crate::name_arena::{NameArena, NameRef};
use crate::path_resolver;
use crate::record_store::RecordStore;

/// Compact file record stored in the in-memory index.
///
/// Layout: exactly 24 bytes.
///   parent_ref  : u64  — 8 bytes (offset 0)
///   size        : u64  — 8 bytes (offset 8)  — file size in bytes (0 for dirs, populated by MFT reader)
///   name_offset : u32  — 4 bytes (offset 16) — byte offset into NameArena
///   name_len    : u16  — 2 bytes (offset 20) — UTF-8 byte count
///   is_dir      : bool — 1 byte  (offset 22)
///   _pad        : u8   — 1 byte  (offset 23)
///
/// Fields are inlined rather than wrapped in NameRef to avoid internal
/// struct padding that would inflate the record further.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FileRecord {
    /// Parent File Reference Number (for path reconstruction).
    pub parent_ref: u64,
    /// File size in bytes. 0 for directories. Populated by MFT size reader.
    pub size: u64,
    /// Byte offset of the file name in VolumeIndex's NameArena.
    pub name_offset: u32,
    /// UTF-8 byte length of the file name.
    pub name_len: u16,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// Explicit padding byte.
    pub _pad: u8,
}

// Compile-time assertion: FileRecord MUST be exactly 24 bytes.
const _: () = assert!(std::mem::size_of::<FileRecord>() == 24);

impl FileRecord {
    /// Construct a [`NameRef`] for use with [`NameArena::get`].
    #[inline]
    pub fn name_ref(&self) -> NameRef {
        NameRef {
            offset: self.name_offset,
            len: self.name_len,
        }
    }
}

/// State of the index for a volume.
#[derive(Clone, Debug)]
pub enum IndexState {
    NotStarted,
    Scanning,
    Ready,
    Error(String),
}

/// The main file index for a single NTFS volume.
pub struct VolumeIndex {
    /// Drive letter, e.g., 'C'.
    pub drive_letter: char,
    /// File Reference Number -> FileRecord.
    pub records: RecordStore,
    /// Reverse index: parent FRN -> child FRNs.
    /// Enables O(subtree) descent for folder size calculation.
    pub children: HashMap<u64, Vec<u64>>,
    /// Contiguous arena storing all file name strings.
    pub names: NameArena,
    /// Last USN processed (for incremental updates).
    pub last_usn: i64,
    /// USN Journal ID (to detect journal resets).
    pub journal_id: u64,
    /// Indexing state.
    pub state: IndexState,
    /// Whether file sizes have been populated from the MFT.
    pub sizes_loaded: bool,
    /// Whether the on-disk binary snapshot needs to be rewritten.
    pub binary_dirty: bool,
    /// FRNs added or modified since the last DB persist.
    pub pending_additions: HashSet<u64>,
    /// FRNs removed since the last DB persist.
    pub pending_removals: HashSet<u64>,
    /// Tracks when each directory was last modified (child created/deleted/renamed).
    /// Used by CheckPathsModified to detect external changes without disk I/O.
    /// Key: parent directory FRN, Value: monotonic instant of last modification.
    pub dir_modified_at: HashMap<u64, std::time::Instant>,
    /// FRNs of files whose size may have changed (detected via USN journal).
    /// The volume indexer drains this set periodically and refreshes sizes
    /// via `FSCTL_GET_NTFS_FILE_RECORD`.
    pub pending_size_refresh: HashSet<u64>,
    /// Extra parent FRNs for hardlinked files.  Key: child FRN, Value:
    /// additional parent FRNs beyond the primary one in `FileRecord.parent_ref`.
    /// Populated during MFT enumeration, consumed by `rebuild_children()`.
    pub hardlink_parents: HashMap<u64, Vec<u64>>,
    /// FRNs that are reparse points (junctions/symlinks/mount points).
    /// Directory traversal must not descend into these, matching the recursive
    /// fallback scan and Explorer's folder size behaviour.
    pub reparse_points: HashSet<u64>,
    /// Whether this volume's hardlink parent relationships are complete.
    /// Old DB caches only persisted a single parent per FRN, so NTFS volumes
    /// loaded from those caches must force one full scan before folder sizes
    /// can match Explorer reliably.
    pub hardlink_data_complete: bool,
    /// Whether reparse-point data was captured and loaded completely.
    pub reparse_data_complete: bool,
}

impl VolumeIndex {
    pub(crate) const DEFAULT_NAME_BYTES_PER_RECORD: usize = 25;

    #[inline]
    fn estimated_child_bucket_capacity(estimated_records: usize) -> usize {
        if estimated_records == 0 {
            0
        } else {
            estimated_records.saturating_mul(2).max(5) / 5
        }
    }

    pub fn empty(drive_letter: char) -> Self {
        Self {
            drive_letter,
            records: RecordStore::new(),
            children: HashMap::new(),
            names: NameArena::with_capacity(0),
            last_usn: 0,
            journal_id: 0,
            state: IndexState::NotStarted,
            sizes_loaded: false,
            binary_dirty: false,
            pending_additions: HashSet::new(),
            pending_removals: HashSet::new(),
            dir_modified_at: HashMap::new(),
            pending_size_refresh: HashSet::new(),
            hardlink_parents: HashMap::new(),
            reparse_points: HashSet::new(),
            hardlink_data_complete: false,
            reparse_data_complete: false,
        }
    }

    pub fn with_capacity(
        drive_letter: char,
        estimated_records: usize,
        estimated_name_bytes: usize,
    ) -> Self {
        let child_capacity = Self::estimated_child_bucket_capacity(estimated_records);

        Self {
            drive_letter,
            records: RecordStore::with_capacity(estimated_records),
            children: HashMap::with_capacity(child_capacity),
            names: NameArena::with_capacity(estimated_name_bytes),
            last_usn: 0,
            journal_id: 0,
            state: IndexState::NotStarted,
            sizes_loaded: false,
            binary_dirty: false,
            pending_additions: HashSet::new(),
            pending_removals: HashSet::new(),
            dir_modified_at: HashMap::new(),
            pending_size_refresh: HashSet::new(),
            hardlink_parents: HashMap::new(),
            reparse_points: HashSet::new(),
            hardlink_data_complete: false,
            reparse_data_complete: false,
        }
    }

    pub fn with_sorted_record_capacity(
        drive_letter: char,
        estimated_records: usize,
        estimated_name_bytes: usize,
    ) -> Self {
        let child_capacity = if estimated_records == 0 {
            0
        } else {
            estimated_records.saturating_div(8).max(1024)
        };

        Self {
            drive_letter,
            records: RecordStore::with_sorted_capacity(estimated_records),
            children: HashMap::with_capacity(child_capacity),
            names: NameArena::with_capacity(estimated_name_bytes),
            last_usn: 0,
            journal_id: 0,
            state: IndexState::NotStarted,
            sizes_loaded: false,
            binary_dirty: false,
            pending_additions: HashSet::new(),
            pending_removals: HashSet::new(),
            dir_modified_at: HashMap::new(),
            pending_size_refresh: HashSet::new(),
            hardlink_parents: HashMap::new(),
            reparse_points: HashSet::new(),
            hardlink_data_complete: false,
            reparse_data_complete: false,
        }
    }

    pub fn with_estimated_records(drive_letter: char, estimated_records: usize) -> Self {
        Self::with_capacity(
            drive_letter,
            estimated_records,
            estimated_records.saturating_mul(Self::DEFAULT_NAME_BYTES_PER_RECORD),
        )
    }

    /// The NTFS root directory's File Reference Number.
    const ROOT_FRN: u64 = 5;

    #[inline]
    fn normalized_parent_bucket(frn: u64, parent_ref: u64) -> u64 {
        if parent_ref == 0 || parent_ref == frn {
            Self::ROOT_FRN
        } else {
            parent_ref
        }
    }

    #[inline]
    fn add_child_edge(&mut self, parent_ref: u64, child_frn: u64) {
        let bucket = Self::normalized_parent_bucket(child_frn, parent_ref);
        let children = self.children.entry(bucket).or_default();
        if !children.contains(&child_frn) {
            children.push(child_frn);
        }
    }

    #[inline]
    fn remove_child_edge(&mut self, parent_ref: u64, child_frn: u64) {
        let bucket = Self::normalized_parent_bucket(child_frn, parent_ref);
        let mut remove_bucket = false;
        if let Some(siblings) = self.children.get_mut(&bucket) {
            if siblings.contains(&child_frn) {
                siblings.retain(|&c| c != child_frn);
                remove_bucket = siblings.is_empty();
            }
        }
        if remove_bucket {
            self.children.remove(&bucket);
        }
    }

    fn sanitize_hardlink_entry(&mut self, frn: u64) {
        let Some(record) = self.records.get(&frn).copied() else {
            self.hardlink_parents.remove(&frn);
            return;
        };

        let mut remove_entry = false;
        if let Some(parents) = self.hardlink_parents.get_mut(&frn) {
            if record.is_dir {
                remove_entry = true;
            } else {
                parents
                    .retain(|&parent| parent != 0 && parent != frn && parent != record.parent_ref);
                parents.sort_unstable();
                parents.dedup();
                remove_entry = parents.is_empty();
            }
        }

        if remove_entry {
            self.hardlink_parents.remove(&frn);
        }
    }

    fn sanitize_hardlink_parents(&mut self) {
        let frns: Vec<u64> = self.hardlink_parents.keys().copied().collect();
        for frn in frns {
            self.sanitize_hardlink_entry(frn);
        }
    }

    #[inline]
    fn set_reparse_state(&mut self, frn: u64, is_reparse: bool) {
        if is_reparse {
            self.reparse_points.insert(frn);
        } else {
            self.reparse_points.remove(&frn);
        }
    }

    /// Insert a file record into the index, storing its name in the arena.
    /// Returns `false` if the arena is full and the record was not inserted.
    ///
    /// **Hardlink handling**: when the same FRN is inserted with a *different*
    /// parent, the old parent's children entry is kept (the file appears under
    /// both parents).  This matches Explorer's behaviour of counting hardlinked
    /// files in every directory that references them.
    fn insert_record_internal(
        &mut self,
        frn: u64,
        name: &str,
        parent_ref: u64,
        is_dir: bool,
        is_reparse: bool,
        track_pending: bool,
    ) -> bool {
        let nr = match self.names.insert(name) {
            Some(nr) => nr,
            None => return false,
        };

        // Determine what to do with the children map based on existing record.
        let old = self.records.get(&frn).map(|r| (r.parent_ref, r.size));

        let preserved_size = old.map_or(0, |(_, s)| s);

        self.records.insert(
            frn,
            FileRecord {
                parent_ref,
                size: preserved_size,
                name_offset: nr.offset,
                name_len: nr.len,
                is_dir,
                _pad: 0,
            },
        );
        self.set_reparse_state(frn, is_reparse);

        match old {
            Some((old_parent, _)) if old_parent == parent_ref => {
                // Same parent re-insert (e.g. long name + 8.3 short name).
                // FRN is already in this parent's children — skip to avoid duplicates.
            }
            Some((old_parent, _)) => {
                // Different parent — this is a hardlink.  Save the OLD parent
                // as an extra so `rebuild_children` can restore both entries.
                let extras = self.hardlink_parents.entry(frn).or_default();
                if !extras.contains(&old_parent) {
                    extras.push(old_parent);
                }
                // Invariant: extras must never contain the current primary
                // parent_ref, otherwise rebuild_children would duplicate the
                // child under that parent.  This can happen when interleaved
                // MFT filename attributes (long + 8.3) ping-pong between
                // two hardlink targets.
                extras.retain(|&p| p != parent_ref);
                self.sanitize_hardlink_entry(frn);
                self.add_child_edge(parent_ref, frn);
            }
            None => {
                // Brand-new record.
                self.add_child_edge(parent_ref, frn);
            }
        }

        if track_pending {
            // Track for incremental SQLite persistence.
            self.pending_removals.remove(&frn);
            self.pending_additions.insert(frn);
            self.binary_dirty = true;
        }
        true
    }

    pub fn insert_record(
        &mut self,
        frn: u64,
        name: &str,
        parent_ref: u64,
        is_dir: bool,
        is_reparse: bool,
    ) -> bool {
        self.insert_record_internal(frn, name, parent_ref, is_dir, is_reparse, true)
    }

    /// Insert a record while building a complete snapshot from MFT/SQLite/FS scan.
    /// Bulk snapshots are persisted explicitly, so per-record pending tracking
    /// would only inflate peak RAM without adding durability.
    pub fn insert_record_untracked(
        &mut self,
        frn: u64,
        name: &str,
        parent_ref: u64,
        is_dir: bool,
        is_reparse: bool,
    ) -> bool {
        self.insert_record_internal(frn, name, parent_ref, is_dir, is_reparse, false)
    }

    /// Insert a new untracked record while reading sorted MFT records.
    /// This avoids the large mutable HashMap overlay used by incremental paths.
    pub fn insert_sorted_record_untracked(
        &mut self,
        frn: u64,
        name: &str,
        parent_ref: u64,
        is_dir: bool,
        is_reparse: bool,
        size: u64,
    ) -> bool {
        let nr = match self.names.insert(name) {
            Some(nr) => nr,
            None => return false,
        };

        let record = FileRecord {
            parent_ref,
            size,
            name_offset: nr.offset,
            name_len: nr.len,
            is_dir,
            _pad: 0,
        };

        match self.records.push_sorted(frn, record) {
            Ok(()) => {
                self.set_reparse_state(frn, is_reparse);
                self.add_child_edge(parent_ref, frn);
                true
            }
            Err(record) => {
                let old = self.records.get(&frn).map(|r| (r.parent_ref, r.size));
                let mut record = record;
                if record.size == 0 {
                    record.size = old.map_or(0, |(_, old_size)| old_size);
                }
                self.records.insert(frn, record);
                self.set_reparse_state(frn, is_reparse);

                match old {
                    Some((old_parent, _)) if old_parent == parent_ref => {}
                    Some((old_parent, _)) => {
                        let extras = self.hardlink_parents.entry(frn).or_default();
                        if !extras.contains(&old_parent) {
                            extras.push(old_parent);
                        }
                        extras.retain(|&p| p != parent_ref);
                        self.sanitize_hardlink_entry(frn);
                        self.add_child_edge(parent_ref, frn);
                    }
                    None => self.add_child_edge(parent_ref, frn),
                }
                true
            }
        }
    }

    /// Remove a file record from the index.
    /// The name bytes remain in the arena as dead space (reclaimed on persist+reload).
    pub fn remove_record(&mut self, frn: u64) {
        if let Some(record) = self.records.remove(&frn) {
            // Remove from primary parent's children list.
            self.remove_child_edge(record.parent_ref, frn);
        }
        self.reparse_points.remove(&frn);
        // Remove from all hardlink extra parents' children lists.
        if let Some(extra_parents) = self.hardlink_parents.remove(&frn) {
            for parent in extra_parents {
                self.remove_child_edge(parent, frn);
            }
        }

        // If it was added since last persist, just undo the add (never reached DB).
        // Otherwise mark for removal from DB.
        if !self.pending_additions.remove(&frn) {
            self.pending_removals.insert(frn);
        }
        self.binary_dirty = true;
    }

    /// Move/rename a file: remove from old parent's children and re-insert
    /// under the new parent.  Used by incremental USN updates when the reason
    /// includes `USN_REASON_RENAME_NEW_NAME`.
    pub fn move_record(
        &mut self,
        frn: u64,
        name: &str,
        new_parent: u64,
        is_dir: bool,
        is_reparse: bool,
    ) -> bool {
        let nr = match self.names.insert(name) {
            Some(nr) => nr,
            None => return false,
        };

        let preserved_size = self.records.get(&frn).map_or(0, |r| r.size);

        // Remove from old primary parent's children (true move, not hardlink).
        if let Some(old_record) = self.records.get(&frn) {
            let old_parent = old_record.parent_ref;
            if old_parent != new_parent {
                self.remove_child_edge(old_parent, frn);
            }
        }

        self.records.insert(
            frn,
            FileRecord {
                parent_ref: new_parent,
                size: preserved_size,
                name_offset: nr.offset,
                name_len: nr.len,
                is_dir,
                _pad: 0,
            },
        );
        self.set_reparse_state(frn, is_reparse);

        // Add to new parent's children if not already there.
        self.sanitize_hardlink_entry(frn);
        self.add_child_edge(new_parent, frn);

        self.pending_removals.remove(&frn);
        self.pending_additions.insert(frn);
        self.binary_dirty = true;
        true
    }

    /// Clear all records and arena data (for full re-scan).
    pub fn clear(&mut self) {
        self.records.clear();
        self.children.clear();
        self.names.clear();
        self.sizes_loaded = false;
        self.binary_dirty = false;
        self.pending_additions.clear();
        self.pending_removals.clear();
        self.dir_modified_at.clear();
        self.pending_size_refresh.clear();
        self.hardlink_parents.clear();
        self.reparse_points.clear();
        self.hardlink_data_complete = false;
        self.reparse_data_complete = false;
    }

    /// Reset change tracking (call after a full `save_volume` persist).
    pub fn clear_pending(&mut self) {
        self.pending_additions.clear();
        self.pending_removals.clear();
    }

    /// SEC: Remove `dir_modified_at` entries older than `max_age` to prevent
    /// unbounded memory growth from long-running incremental USN updates.
    pub fn prune_old_modifications(&mut self, max_age: Duration) {
        let now = std::time::Instant::now();
        self.dir_modified_at.retain(|_frn, ts| {
            now.checked_duration_since(*ts)
                .map(|age| age <= max_age)
                .unwrap_or(false)
        });
    }

    /// Compact the arena: rebuild with only the names referenced by current
    /// records.  Eliminates dead space from duplicate MFT name attributes
    /// (long name + 8.3 short name for the same FRN) and incremental overwrites.
    pub fn compact_arena(&mut self) {
        let mut new_arena = NameArena::with_capacity(self.records.len() * 25);
        for record in self.records.values_mut() {
            let name = self.names.get(record.name_ref());
            // This cannot fail: we are rebuilding from existing names that
            // already fit in the old arena (same size or smaller).
            let nr = new_arena
                .insert(name)
                .expect("compact_arena: rebuilt data must fit");
            record.name_offset = nr.offset;
            record.name_len = nr.len;
        }
        self.names = new_arena;
        self.shrink_to_fit();
    }

    /// Release excess capacity after a bulk load/scan has stabilized.
    pub fn shrink_to_fit(&mut self) {
        self.records.shrink_to_fit();
        self.children.shrink_to_fit();
        self.names.shrink_to_fit();
        self.pending_additions.shrink_to_fit();
        self.pending_removals.shrink_to_fit();
        self.dir_modified_at.shrink_to_fit();
        self.pending_size_refresh.shrink_to_fit();
        self.hardlink_parents.shrink_to_fit();
        for parents in self.hardlink_parents.values_mut() {
            parents.shrink_to_fit();
        }
        self.reparse_points.shrink_to_fit();
    }

    /// Rebuild the `children` reverse index from `records` and `hardlink_parents`.
    /// Call after bulk operations (load from DB, compaction) to ensure consistency.
    pub fn rebuild_children(&mut self) {
        self.sanitize_hardlink_parents();
        let mut rebuilt: HashMap<u64, Vec<u64>> = HashMap::with_capacity(self.records.len() / 3);
        // Primary parent from each record.
        for (&frn, record) in &self.records {
            rebuilt
                .entry(Self::normalized_parent_bucket(frn, record.parent_ref))
                .or_default()
                .push(frn);
        }
        // Extra parents from hardlinks — the file appears in these
        // directories too, matching Explorer's recursive size counting.
        // Skip entries that match the primary parent_ref (invariant should
        // already prevent this, but guard against stale DB data).
        for (&frn, extra_parents) in &self.hardlink_parents {
            let primary = self
                .records
                .get(&frn)
                .map(|r| Self::normalized_parent_bucket(frn, r.parent_ref));
            for &parent in extra_parents {
                let bucket = Self::normalized_parent_bucket(frn, parent);
                if Some(bucket) != primary {
                    rebuilt.entry(bucket).or_default().push(frn);
                }
            }
        }
        self.children = rebuilt
            .into_iter()
            .map(|(parent, mut child_frns)| {
                child_frns.sort_unstable();
                child_frns.dedup();
                child_frns.shrink_to_fit();
                (parent, child_frns)
            })
            .collect();
        self.children.shrink_to_fit();
    }

    /// Compute recursive subtree totals for a directory using the `children`
    /// reverse index for O(subtree) traversal.
    ///
    /// Returns `(total_size, file_count, folder_count)` where `folder_count`
    /// excludes the queried root directory itself.
    ///
    /// All directories are traversed, including reparse-point directories.
    /// Junction/symlink FRNs have zero children in the MFT (their content
    /// lives under the *target* FRN) so they add nothing.  Cloud-reparse
    /// folders (OneDrive) DO have real children and must be counted to
    /// match Explorer.  `visited_dirs` prevents cycles.
    pub fn folder_tree_summary(&self, dir_frn: u64) -> (u64, u64, u64, u64) {
        let mut total_size: u64 = 0;
        let mut file_count: u64 = 0;
        let mut folder_count: u64 = 0;
        let mut zero_size_count: u64 = 0;
        let mut stack = vec![dir_frn];
        let mut visited_dirs = HashSet::with_capacity(256);
        while let Some(frn) = stack.pop() {
            if !visited_dirs.insert(frn) {
                continue;
            }
            if let Some(child_frns) = self.children.get(&frn) {
                for &child_frn in child_frns {
                    if let Some(record) = self.records.get(&child_frn) {
                        if record.is_dir {
                            folder_count += 1;
                            stack.push(child_frn);
                        } else {
                            total_size = total_size.saturating_add(record.size);
                            file_count += 1;
                            if record.size == 0 {
                                zero_size_count += 1;
                            }
                        }
                    }
                }
            }
        }

        (total_size, file_count, folder_count, zero_size_count)
    }

    /// Diagnostic variant of `folder_size_sum` that also computes a unique-by-FRN
    /// total for files. This helps distinguish true tree duplication from mere
    /// multiple child edges to the same file within one queried subtree.
    pub fn folder_size_sum_unique_files(&self, dir_frn: u64) -> (u64, u64, u64) {
        let mut total_size: u64 = 0;
        let mut file_count: u64 = 0;
        let mut duplicate_hits: u64 = 0;
        let mut stack = vec![dir_frn];
        let mut visited_dirs = HashSet::with_capacity(256);
        let mut seen_files = HashSet::with_capacity(1024);

        while let Some(frn) = stack.pop() {
            if !visited_dirs.insert(frn) {
                continue;
            }
            if let Some(child_frns) = self.children.get(&frn) {
                for &child_frn in child_frns {
                    if let Some(record) = self.records.get(&child_frn) {
                        if record.is_dir {
                            stack.push(child_frn);
                        } else if seen_files.insert(child_frn) {
                            total_size = total_size.saturating_add(record.size);
                            file_count += 1;
                        } else {
                            duplicate_hits += 1;
                        }
                    }
                }
            }
        }

        (total_size, file_count, duplicate_hits)
    }

    /// Collect up to `limit` unique file FRNs under `dir_frn` whose recorded
    /// size is still zero. This is used to target on-demand repair of stale
    /// cache entries without rescanning the entire volume.
    pub fn collect_zero_size_file_frns_in_subtree(&self, dir_frn: u64, limit: usize) -> Vec<u64> {
        if limit == 0 {
            return Vec::new();
        }

        let mut zero_size_frns = Vec::with_capacity(limit.min(256));
        let mut stack = vec![dir_frn];
        let mut visited_dirs = HashSet::with_capacity(256);
        let mut seen_files = HashSet::with_capacity(limit.min(1024));

        while let Some(frn) = stack.pop() {
            if !visited_dirs.insert(frn) {
                continue;
            }

            if let Some(child_frns) = self.children.get(&frn) {
                for &child_frn in child_frns {
                    let Some(record) = self.records.get(&child_frn) else {
                        continue;
                    };

                    if record.is_dir {
                        stack.push(child_frn);
                        continue;
                    }

                    if record.size == 0 && seen_files.insert(child_frn) {
                        zero_size_frns.push(child_frn);
                        if zero_size_frns.len() >= limit {
                            return zero_size_frns;
                        }
                    }
                }
            }
        }

        zero_size_frns
    }

    /// Collect unique file FRNs under `dir_frn`, returning `None` if the
    /// subtree exceeds `limit`. Used for targeted live size refreshes on small
    /// folders without making large FolderSize requests unexpectedly expensive.
    pub fn collect_file_frns_in_subtree_limited(
        &self,
        dir_frn: u64,
        limit: usize,
    ) -> Option<Vec<u64>> {
        let mut file_frns = Vec::with_capacity(limit.min(256));
        let mut stack = vec![dir_frn];
        let mut visited_dirs = HashSet::with_capacity(256);
        let mut seen_files = HashSet::with_capacity(limit.min(1024));

        while let Some(frn) = stack.pop() {
            if !visited_dirs.insert(frn) {
                continue;
            }

            if let Some(child_frns) = self.children.get(&frn) {
                for &child_frn in child_frns {
                    let Some(record) = self.records.get(&child_frn) else {
                        continue;
                    };

                    if record.is_dir {
                        stack.push(child_frn);
                    } else if seen_files.insert(child_frn) {
                        if file_frns.len() >= limit {
                            return None;
                        }
                        file_frns.push(child_frn);
                    }
                }
            }
        }

        Some(file_frns)
    }

    /// Report estimated memory usage: (arena_used, arena_capacity, records_estimate).
    pub fn memory_usage(&self) -> (usize, usize, usize) {
        let arena_used = self.names.len();
        let arena_cap = self.names.capacity();
        let records_est = self.records.estimated_heap_bytes();
        (arena_used, arena_cap, records_est)
    }

    /// Bytes in the name arena that are still referenced by live records.
    pub fn referenced_name_bytes(&self) -> usize {
        self.records
            .iter()
            .map(|(_, record)| record.name_len as usize)
            .sum()
    }

    /// Resolve a filesystem path (e.g., `C:\Users\foo`) to the FRN of the
    /// directory, walking path components through the index.
    /// Returns `None` if any component is not found in the records.
    pub fn resolve_path_to_frn(&self, path: &str) -> Option<u64> {
        // Expect "X:\..." where X matches our drive letter
        let path = path.strip_prefix(|c: char| c.eq_ignore_ascii_case(&self.drive_letter))?;
        let path = path.strip_prefix(":\\")?;

        if path.is_empty() {
            return Some(Self::ROOT_FRN);
        }

        let mut current_frn = Self::ROOT_FRN;

        for component in path.split('\\') {
            if component.is_empty() {
                continue;
            }
            // Use the reverse children index instead of scanning the entire
            // volume. This keeps path resolution proportional to the current
            // directory fanout, not to the total file count on the drive.
            let child_frn = self.children.get(&current_frn).and_then(|child_frns| {
                child_frns.iter().copied().find(|child_frn| {
                    self.records
                        .get(child_frn)
                        .map(|record| {
                            // Directory path resolution must only traverse
                            // directory records. This avoids selecting stale
                            // non-directory siblings in ambiguous indexes and
                            // returning incorrect subtree totals.
                            record.is_dir
                                && self
                                    .names
                                    .get(record.name_ref())
                                    .eq_ignore_ascii_case(component)
                        })
                        .unwrap_or(false)
                })
            });
            current_frn = child_frn?;
        }

        Some(current_frn)
    }
}

/// Search result item returned from a search query.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub size: u64,
}

pub struct SearchPage {
    pub items: Vec<SearchResult>,
    pub has_more: bool,
    #[allow(dead_code)]
    pub total_matches: Option<usize>,
}

/// Case-insensitive substring search without allocation.
///
/// Checks if `needle_lower` (already lowercased) appears as a contiguous
/// substring inside `haystack` (case-insensitively).
/// Uses byte-level ASCII fast-path with char-level fallback for non-ASCII.
#[inline]
fn contains_case_insensitive(haystack: &str, needle_lower: &str) -> bool {
    let needle_bytes = needle_lower.as_bytes();
    let haystack_bytes = haystack.as_bytes();

    if needle_bytes.is_empty() {
        return true;
    }

    // Fast path: if both are pure ASCII, do byte-level comparison
    if haystack.is_ascii() && needle_lower.is_ascii() {
        if haystack_bytes.len() < needle_bytes.len() {
            return false;
        }
        let limit = haystack_bytes.len() - needle_bytes.len() + 1;
        'outer: for i in 0..limit {
            for j in 0..needle_bytes.len() {
                if haystack_bytes[i + j].to_ascii_lowercase() != needle_bytes[j] {
                    continue 'outer;
                }
            }
            return true;
        }
        return false;
    }

    // Slow path: full Unicode lowercase via char iteration
    let haystack_lower: String = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    haystack_lower.contains(needle_lower)
}

/// Returns `true` if all whitespace-separated tokens in `tokens` appear
/// (case-insensitively) somewhere in `haystack`.
/// Single-token slices behave identically to the old `contains_case_insensitive` call.
#[inline]
fn matches_all_tokens(haystack: &str, tokens: &[&str]) -> bool {
    tokens
        .iter()
        .all(|token| contains_case_insensitive(haystack, token))
}

/// Search the indices for files matching a query string.
/// Returns one page (`offset`, `limit`) of matching records with resolved paths.
/// Enforces a time limit to avoid holding locks indefinitely on cold memory.
///
/// Uses SIMD-accelerated `memchr::memmem` over a reusable lowercased scratch
/// buffer for ASCII file names, avoiding a permanent lowercased copy of the
/// entire name arena. Non-ASCII names use the Unicode-safe fallback.
///
/// Each volume's read lock is acquired independently for the duration of
/// scanning that single volume only — see `volume_indices` (F5.4). A USN
/// writer on volume D no longer blocks a search reading volume C.
pub fn search_page(
    handles: &[crate::volume_indices::VolumeIndexHandle],
    query: &str,
    offset: usize,
    limit: usize,
) -> SearchPage {
    if query.is_empty() || limit == 0 {
        return SearchPage {
            items: Vec::new(),
            has_more: false,
            total_matches: Some(0),
        };
    }

    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();
    if tokens.is_empty() {
        return SearchPage {
            items: Vec::new(),
            has_more: false,
            total_matches: Some(0),
        };
    }

    // Pre-build memmem finders for SIMD search (cheaply reused across volumes).
    let finders: Vec<memchr::memmem::Finder<'_>> = tokens
        .iter()
        .map(|t| memchr::memmem::Finder::new(t.as_bytes()))
        .collect();
    let tokens_are_ascii = tokens.iter().all(|t| t.is_ascii());
    let mut lowercase_scratch = Vec::with_capacity(512);

    let mut items = Vec::with_capacity(limit.min(1000));
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1_500);
    let mut scanned: u64 = 0;
    let mut matched_after_filters: usize = 0;
    let mut timed_out = false;

    for handle in handles {
        let index = handle.read();
        if !matches!(index.state, IndexState::Ready) {
            continue;
        }
        let mut dir_path_cache = HashMap::new();

        for (&frn, record) in &index.records {
            scanned += 1;
            if scanned.is_multiple_of(50_000) && std::time::Instant::now() > deadline {
                eprintln!(
                    "[SEARCH] Time limit reached after scanning {} records, returning {} partial results",
                    scanned, items.len()
                );
                timed_out = true;
                break;
            }

            let name = index.names.get(record.name_ref());
            let matches = if tokens_are_ascii && name.is_ascii() {
                lowercase_scratch.clear();
                lowercase_scratch.extend_from_slice(name.as_bytes());
                lowercase_scratch.make_ascii_lowercase();
                finders
                    .iter()
                    .all(|finder| finder.find(&lowercase_scratch).is_some())
            } else {
                matches_all_tokens(name, &tokens)
            };

            if matches {
                if let Some(full_path) =
                    path_resolver::resolve_path_cached(frn, &index, &mut dir_path_cache)
                {
                    if matched_after_filters < offset {
                        matched_after_filters += 1;
                        continue;
                    }

                    if items.len() >= limit {
                        return SearchPage {
                            items,
                            has_more: true,
                            total_matches: None,
                        };
                    }

                    items.push(SearchResult {
                        name: name.to_owned(),
                        full_path,
                        is_dir: record.is_dir,
                        size: record.size,
                    });
                    matched_after_filters += 1;
                }
            }
        }

        if timed_out {
            break;
        }
    }

    SearchPage {
        items,
        has_more: timed_out,
        total_matches: if timed_out {
            None
        } else {
            Some(matched_after_filters)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{search_page, IndexState, VolumeIndex};
    use crate::volume_indices::handle_from;

    #[test]
    fn folder_size_includes_reparse_children() {
        let mut index = VolumeIndex::empty('C');
        let root = 5u64;

        // "real" dir (non-reparse) with a file
        assert!(index.insert_record(10, "real", root, true, false));
        // "junction" dir (reparse) — in real NTFS, junction FRNs have
        // no children in the MFT, but this test verifies that even if
        // children exist (as with OneDrive cloud folders), they are counted.
        assert!(index.insert_record(11, "junction", root, true, true));
        assert!(index.insert_record(12, "root-file.bin", root, false, false));
        assert!(index.insert_record(20, "real-file.bin", 10, false, false));
        assert!(index.insert_record(21, "junction-file.bin", 11, false, false));

        index.records.get_mut(&12).unwrap().size = 1;
        index.records.get_mut(&20).unwrap().size = 10;
        index.records.get_mut(&21).unwrap().size = 100;

        // Reparse children are now included (needed for OneDrive cloud folders).
        let (raw_total, raw_count, raw_folders, _zero) = index.folder_tree_summary(root);
        assert_eq!((raw_total, raw_count, raw_folders), (111, 3, 2));

        let (unique_total, unique_count, duplicate_hits) = index.folder_size_sum_unique_files(root);
        assert_eq!((unique_total, unique_count, duplicate_hits), (111, 3, 0));
    }

    #[test]
    fn folder_size_visited_dirs_prevents_cycles() {
        let mut index = VolumeIndex::empty('C');
        let root = 5u64;

        // Create a dir structure where the same FRN could appear twice
        // (e.g., hardlink-like scenario). visited_dirs prevents re-counting.
        assert!(index.insert_record(10, "dir_a", root, true, false));
        assert!(index.insert_record(20, "file.bin", 10, false, false));
        index.records.get_mut(&20).unwrap().size = 42;

        let (total, count, folder_count, _zero) = index.folder_tree_summary(root);
        assert_eq!((total, count, folder_count), (42, 1, 1));
    }

    #[test]
    fn collect_zero_size_file_frns_in_subtree_is_unique_and_respects_limit() {
        let mut index = VolumeIndex::empty('C');
        let root = 5u64;

        assert!(index.insert_record(10, "folder", root, true, false));
        assert!(index.insert_record(11, "nested", 10, true, false));
        assert!(index.insert_record(20, "zero-a.bin", 10, false, false));
        assert!(index.insert_record(21, "nonzero.bin", 11, false, false));
        assert!(index.insert_record(22, "zero-b.bin", 11, false, false));
        assert!(index.insert_record(22, "zero-b.bin", root, false, false));

        index.records.get_mut(&21).unwrap().size = 7;

        let mut collected = index.collect_zero_size_file_frns_in_subtree(root, 10);
        collected.sort_unstable();
        assert_eq!(collected, vec![20, 22]);

        let limited = index.collect_zero_size_file_frns_in_subtree(root, 1);
        assert_eq!(limited.len(), 1);
    }

    #[test]
    fn collect_file_frns_in_subtree_limited_is_unique_and_reports_over_limit() {
        let mut index = VolumeIndex::empty('C');
        let root = 5u64;

        assert!(index.insert_record(10, "folder", root, true, false));
        assert!(index.insert_record(20, "a.bin", 10, false, false));
        assert!(index.insert_record(21, "b.bin", 10, false, false));
        assert!(index.insert_record(21, "b.bin", root, false, false));

        let mut collected = index
            .collect_file_frns_in_subtree_limited(root, 2)
            .expect("subtree should fit limit");
        collected.sort_unstable();
        assert_eq!(collected, vec![20, 21]);

        assert!(index
            .collect_file_frns_in_subtree_limited(root, 1)
            .is_none());
    }

    #[test]
    fn search_page_matches_ascii_case_insensitively_without_lowered_arena() {
        let mut index = VolumeIndex::empty('C');
        index.state = IndexState::Ready;
        assert!(index.insert_record(10, "Annual Report.TXT", 5, false, false));
        assert!(index.insert_record(11, "other.bin", 5, false, false));

        let handle = handle_from(index);
        let page = search_page(&[handle], "report", 0, 10);

        assert_eq!(page.total_matches, Some(1));
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].name, "Annual Report.TXT");
        assert_eq!(page.items[0].full_path, r"C:\Annual Report.TXT");
    }

    #[test]
    fn search_page_preserves_unicode_case_insensitive_fallback() {
        let mut index = VolumeIndex::empty('C');
        index.state = IndexState::Ready;
        assert!(index.insert_record(10, "Relatório Café.txt", 5, false, false));

        let handle = handle_from(index);
        let page = search_page(&[handle], "relatório café", 0, 10);

        assert_eq!(page.total_matches, Some(1));
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].name, "Relatório Café.txt");
    }

    #[test]
    fn untracked_insert_preserves_index_without_pending_growth() {
        let mut index = VolumeIndex::empty('C');

        assert!(index.insert_record_untracked(10, "folder", 5, true, false));
        assert!(index.insert_record_untracked(20, "file.bin", 10, false, false));

        assert!(index.pending_additions.is_empty());
        assert!(index.pending_removals.is_empty());
        assert!(!index.binary_dirty);
        assert_eq!(index.resolve_path_to_frn(r"C:\folder"), Some(10));
        assert_eq!(index.folder_tree_summary(10).1, 1);
    }

    #[test]
    fn sorted_untracked_insert_builds_compact_store_without_pending_growth() {
        let mut index = VolumeIndex::with_sorted_record_capacity('C', 2, 64);

        assert!(index.insert_sorted_record_untracked(10, "folder", 5, true, false, 0));
        assert!(index.insert_sorted_record_untracked(20, "file.bin", 10, false, false, 42));

        assert!(index.pending_additions.is_empty());
        assert!(index.pending_removals.is_empty());
        assert!(!index.binary_dirty);
        assert!(index.records.compact_sorted_slices().is_some());
        assert_eq!(index.folder_tree_summary(10).0, 42);
    }

    #[test]
    fn compact_arena_preserves_children_and_hardlinks() {
        let mut index = VolumeIndex::empty('C');

        assert!(index.insert_record(10, "folder", 5, true, false));
        assert!(index.insert_record(11, "other", 5, true, false));
        assert!(index.insert_record(20, "file.bin", 10, false, false));
        index.records.get_mut(&20).unwrap().size = 7;
        assert!(index.insert_record(20, "file.bin", 11, false, false));
        index.records.get_mut(&20).unwrap().size = 7;

        index.compact_arena();

        let folder_summary = index.folder_tree_summary(10);
        let other_summary = index.folder_tree_summary(11);
        assert_eq!((folder_summary.0, folder_summary.1), (7, 1));
        assert_eq!((other_summary.0, other_summary.1), (7, 1));
    }

    #[test]
    fn resolve_path_to_frn_prefers_directory_records() {
        let mut index = VolumeIndex::empty('C');

        // Simulate a stale/ambiguous index bucket where a file and a
        // directory share the same displayed name under one parent.
        assert!(index.insert_record(10, "Sample", 5, false, false));
        assert!(index.insert_record(11, "Sample", 5, true, false));
        assert!(index.insert_record(12, "nested.bin", 11, false, false));

        let resolved = index.resolve_path_to_frn(r"C:\Sample");
        assert_eq!(resolved, Some(11));

        let (total, files, folders, _zero) = index.folder_tree_summary(11);
        assert_eq!((total, files, folders), (0, 1, 0));
    }
}
