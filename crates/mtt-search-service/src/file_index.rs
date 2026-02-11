use std::collections::HashMap;

use crate::path_resolver;

/// Compact file record stored in the in-memory index.
#[derive(Clone, Debug)]
pub struct FileRecord {
    /// File name (just the name, not full path).
    pub name: String,
    /// Pre-computed lowercase name for fast search matching.
    pub name_lower: String,
    /// Parent File Reference Number (for path reconstruction).
    pub parent_ref: u64,
    /// Whether this is a directory.
    pub is_dir: bool,
    /// File size in bytes (0 for directories, also 0 from USN enum).
    pub size: u64,
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
            last_usn: 0,
            journal_id: 0,
            state: IndexState::NotStarted,
        }
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

/// Search the indices for files matching a query string.
/// Returns up to `max_results` matching records with their resolved paths.
/// Enforces a time limit to avoid holding locks indefinitely on cold memory.
pub fn search(indices: &[VolumeIndex], query: &str, max_results: usize) -> Vec<SearchResult> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::with_capacity(max_results.min(1000));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut scanned: u64 = 0;

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
                    results.len()
                );
                return results;
            }

            if record.name_lower.contains(&query_lower) {
                if let Some(full_path) = path_resolver::resolve_path(frn, index) {
                    results.push(SearchResult {
                        name: record.name.clone(),
                        full_path,
                        is_dir: record.is_dir,
                        size: record.size,
                    });

                    if results.len() >= max_results {
                        return results;
                    }
                }
            }
        }
    }

    results
}
