use std::collections::HashMap;

use crate::name_arena::{NameArena, NameRef};
use crate::path_resolver;

/// Compact file record stored in the in-memory index.
///
/// Layout: exactly 16 bytes (down from 72 + heap).
///   parent_ref  : u64  — 8 bytes (offset 0)
///   name_offset : u32  — 4 bytes (offset 8)  — byte offset into NameArena
///   name_len    : u16  — 2 bytes (offset 12) — UTF-8 byte count
///   is_dir      : bool — 1 byte  (offset 14)
///   _pad        : u8   — 1 byte  (offset 15)
///
/// Fields are inlined rather than wrapped in NameRef to avoid internal
/// struct padding that would inflate the record to 24 bytes.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FileRecord {
    /// Parent File Reference Number (for path reconstruction).
    pub parent_ref: u64,
    /// Byte offset of the file name in VolumeIndex's NameArena.
    pub name_offset: u32,
    /// UTF-8 byte length of the file name.
    pub name_len: u16,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// Explicit padding byte.
    pub _pad: u8,
}

// Compile-time assertion: FileRecord MUST be exactly 16 bytes.
const _: () = assert!(std::mem::size_of::<FileRecord>() == 16);

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
    pub records: HashMap<u64, FileRecord>,
    /// Contiguous arena storing all file name strings.
    pub names: NameArena,
    /// Last USN processed (for incremental updates).
    pub last_usn: i64,
    /// USN Journal ID (to detect journal resets).
    pub journal_id: u64,
    /// Indexing state.
    pub state: IndexState,
}

impl VolumeIndex {
    pub fn new(drive_letter: char) -> Self {
        Self {
            drive_letter,
            records: HashMap::with_capacity(500_000),
            // Pre-allocate ~12.5 MB for names (500K files × ~25 bytes avg).
            names: NameArena::with_capacity(500_000 * 25),
            last_usn: 0,
            journal_id: 0,
            state: IndexState::NotStarted,
        }
    }

    /// Insert a file record into the index, storing its name in the arena.
    pub fn insert_record(&mut self, frn: u64, name: &str, parent_ref: u64, is_dir: bool) {
        let nr = self.names.insert(name);
        self.records.insert(
            frn,
            FileRecord {
                parent_ref,
                name_offset: nr.offset,
                name_len: nr.len,
                is_dir,
                _pad: 0,
            },
        );
    }

    /// Remove a file record from the index.
    /// The name bytes remain in the arena as dead space (reclaimed on persist+reload).
    pub fn remove_record(&mut self, frn: u64) {
        self.records.remove(&frn);
    }

    /// Clear all records and arena data (for full re-scan).
    pub fn clear(&mut self) {
        self.records.clear();
        self.names.clear();
    }

    /// Compact the arena: rebuild with only the names referenced by current
    /// records.  Eliminates dead space from duplicate MFT name attributes
    /// (long name + 8.3 short name for the same FRN) and incremental overwrites.
    pub fn compact_arena(&mut self) {
        let mut new_arena = NameArena::with_capacity(self.records.len() * 25);
        for record in self.records.values_mut() {
            let name = self.names.get(record.name_ref());
            let nr = new_arena.insert(name);
            record.name_offset = nr.offset;
            record.name_len = nr.len;
        }
        self.names = new_arena;
        self.names.shrink_to_fit();
    }

    /// Report estimated memory usage: (arena_used, arena_capacity, hashmap_estimate).
    pub fn memory_usage(&self) -> (usize, usize, usize) {
        let arena_used = self.names.len();
        let arena_cap = self.names.capacity();
        let slot_size = std::mem::size_of::<u64>() + std::mem::size_of::<FileRecord>() + 1;
        let map_est = self.records.capacity() * slot_size;
        (arena_used, arena_cap, map_est)
    }
}

/// Search result item returned from a search query.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
}

pub struct SearchPage {
    pub items: Vec<SearchResult>,
    pub has_more: bool,
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

/// Search the indices for files matching a query string.
/// Returns one page (`offset`, `limit`) of matching records with resolved paths.
/// Enforces a time limit to avoid holding locks indefinitely on cold memory.
pub fn search_page(
    indices: &[VolumeIndex],
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
    let mut items = Vec::with_capacity(limit.min(1000));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut scanned: u64 = 0;
    let mut matched_after_filters: usize = 0;
    let mut timed_out = false;

    for index in indices {
        if !matches!(index.state, IndexState::Ready) {
            continue;
        }

        for (&frn, record) in &index.records {
            // Check deadline every 50K records to avoid Instant::now() overhead
            scanned += 1;
            if scanned % 50_000 == 0 && std::time::Instant::now() > deadline {
                eprintln!(
                    "[SEARCH] Time limit reached after scanning {} records, returning {} partial results",
                    scanned,
                    items.len()
                );
                timed_out = true;
                break;
            }

            let name = index.names.get(record.name_ref());

            if contains_case_insensitive(name, &query_lower) {
                if let Some(full_path) = path_resolver::resolve_path(frn, index) {
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
