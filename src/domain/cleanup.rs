//! Cleanup feature domain types
//!
//! Pure data types for the file/folder cleanup feature. The scanner is
//! read-only; deletion goes through the shell operation worker. Safety rules:
//! only built-in category roots are scanned, reparse points are never
//! traversed or deleted, and an age filter is applied at scan time and
//! re-validated at delete time.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Built-in cleanup categories. Each variant maps to curated, rebuildable
/// junk locations. Never extend this with arbitrary user input in v1.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum CleanupCategoryId {
    UserTemp,
    SystemTemp,
    DirectXShaderCache,
    ErrorReports,
    CrashDumps,
    ExplorerThumbnailCache,
}

impl CleanupCategoryId {
    /// Fixed display order for the review UI.
    pub const ALL: [CleanupCategoryId; 6] = [
        CleanupCategoryId::UserTemp,
        CleanupCategoryId::SystemTemp,
        CleanupCategoryId::DirectXShaderCache,
        CleanupCategoryId::ErrorReports,
        CleanupCategoryId::CrashDumps,
        CleanupCategoryId::ExplorerThumbnailCache,
    ];
}

/// A built-in cleanup category: the roots it may scan (first-level entries
/// only) and, optionally, file-name patterns restricted to the root itself.
#[derive(Clone, Debug)]
pub struct CleanupCategory {
    pub id: CleanupCategoryId,
    pub roots: Vec<PathBuf>,
    /// When set, only first-level files matching one of these case-insensitive
    /// glob-ish prefixes/patterns are candidates. Directories are skipped.
    pub file_patterns: Option<Vec<&'static str>>,
    /// Category roots that typically need elevation. The scanner still tries;
    /// access errors are reported as skipped, never fatal.
    pub requires_admin: bool,
}

impl CleanupCategory {
    /// True when `path` is lexically inside one of this category's roots
    /// (after normalization performed by the scanner). This is the final
    /// containment guard before a candidate is accepted.
    pub fn contains_path(&self, path: &std::path::Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }
}

/// One deletable first-level entry found by the scan.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CleanupCandidate {
    pub path: PathBuf,
    pub category_id: CleanupCategoryId,
    /// Total bytes of the entry (file size, or recursive folder size when
    /// computable; 0 when the folder could not be fully measured).
    pub size: u64,
    pub is_dir: bool,
    /// Last-modified time as unix seconds. For folders this is the folder's
    /// own mtime, not its contents'.
    pub modified_unix: u64,
}

/// Mutable scan progress shared with the UI thread via `Arc<Mutex<..>>`.
#[derive(Clone, Debug, Default)]
pub struct CleanupScanProgress {
    pub current_path: String,
    pub entries_seen: u64,
    pub bytes_found: u64,
}

pub type SharedCleanupScanProgress = std::sync::Arc<std::sync::Mutex<CleanupScanProgress>>;

/// Lifecycle of the cleanup modal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupPhase {
    Idle,
    Scanning,
    Review,
    Deleting,
    Completed,
    Failed(String),
}

/// Deletion mode for a cleanup run. Recycle-bin is the default; permanent
/// deletion is an opt-in advanced toggle behind an extra confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupDeleteMode {
    RecycleBin,
    Permanent,
}

impl CleanupDeleteMode {
    pub fn preference_value(self) -> &'static str {
        match self {
            CleanupDeleteMode::RecycleBin => "recycle_bin",
            CleanupDeleteMode::Permanent => "permanent",
        }
    }

    pub fn from_preference_value(value: &str) -> Option<Self> {
        match value {
            "recycle_bin" => Some(CleanupDeleteMode::RecycleBin),
            "permanent" => Some(CleanupDeleteMode::Permanent),
            _ => None,
        }
    }
}

/// Per-item progress published by the delete worker.
#[derive(Clone, Debug)]
pub struct CleanupDeleteProgress {
    pub completed: usize,
    pub total: usize,
    pub current_name: Option<String>,
    pub failed_count: usize,
}

/// Final result of a cleanup run, also used for the persisted history entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CleanupRunSummary {
    pub deleted_count: usize,
    pub deleted_bytes: u64,
    /// (path, reason) for items that were not deleted.
    pub skipped: Vec<(String, String)>,
    pub aborted: bool,
    pub permanent: bool,
    /// Unix seconds when the run finished.
    pub finished_at_unix: u64,
}

impl CleanupRunSummary {
    /// Serializes the history ring (most recent last) to the compact JSON
    /// stored in the `cleanup_history` preference. Keeps at most
    /// `CLEANUP_HISTORY_CAP` entries and strips byte-heavy detail (sizes are
    /// kept; paths are kept as strings — this is a local audit trail).
    pub fn encode_history(history: &[CleanupRunSummary]) -> String {
        const CLEANUP_HISTORY_CAP: usize = 20;
        let start = history.len().saturating_sub(CLEANUP_HISTORY_CAP);
        serde_json::to_string(&&history[start..]).unwrap_or_else(|_| "[]".to_string())
    }

    pub fn decode_history(json: &str) -> Vec<CleanupRunSummary> {
        serde_json::from_str::<Vec<CleanupRunSummary>>(json).unwrap_or_default()
    }

    /// Sums deleted bytes of the candidates the run accepted, using the scan
    /// sizes recorded before deletion.
    pub fn deleted_bytes_from(
        deleted: &[PathBuf],
        sizes: &HashMap<PathBuf, u64>,
    ) -> u64 {
        deleted
            .iter()
            .map(|path| sizes.get(path).copied().unwrap_or(0))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_mode_round_trips_preference_value() {
        assert_eq!(
            CleanupDeleteMode::from_preference_value(
                CleanupDeleteMode::RecycleBin.preference_value()
            ),
            Some(CleanupDeleteMode::RecycleBin)
        );
        assert_eq!(
            CleanupDeleteMode::from_preference_value(
                CleanupDeleteMode::Permanent.preference_value()
            ),
            Some(CleanupDeleteMode::Permanent)
        );
        assert_eq!(CleanupDeleteMode::from_preference_value("bogus"), None);
    }

    #[test]
    fn history_round_trips_and_caps_entries() {
        let history: Vec<CleanupRunSummary> = (0..30)
            .map(|i| CleanupRunSummary {
                deleted_count: i,
                deleted_bytes: 0,
                skipped: vec![("file.txt".to_string(), "locked".to_string())],
                aborted: false,
                permanent: false,
                finished_at_unix: i as u64,
            })
            .collect();
        let encoded = CleanupRunSummary::encode_history(&history);
        let decoded = CleanupRunSummary::decode_history(&encoded);
        assert_eq!(decoded.len(), 20);
        assert_eq!(decoded.last().unwrap().deleted_count, 29);
    }

    #[test]
    fn history_decode_tolerates_garbage() {
        assert!(CleanupRunSummary::decode_history("not json").is_empty());
    }

    #[test]
    fn deleted_bytes_sums_known_sizes_only() {
        let mut sizes = HashMap::new();
        sizes.insert(PathBuf::from("a"), 10_u64);
        sizes.insert(PathBuf::from("b"), 5_u64);
        let deleted = vec![PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("c")];
        assert_eq!(CleanupRunSummary::deleted_bytes_from(&deleted, &sizes), 15);
    }
}
