//! Exact duplicates: size → validated 64 KiB prefix → full BLAKE3 confirmation.
//! Partial hashes never confirm large files. Scanning never modifies/downloads
//! files; comparison offers separately confirmed native Windows deletion.

use std::path::PathBuf;
use std::sync::Arc;

mod candidates;
mod deletion;
mod pipeline;
mod reader;
mod report;
mod review;
mod session;
mod workers;

pub use candidates::group_by_size;
pub use deletion::{DeleteFailure, DeleteProgress, DeleteStage, DeleteStatus, DuplicateDeletion};
pub use report::compute_recoverable;
pub use review::{CopyStatus, DuplicateReview, PreviewStatus};
pub use session::DuplicateSession;

pub const DEFAULT_MIN_SIZE: u64 = 1024 * 1024;
pub(super) const PARTIAL_BYTES: u64 = 64 * 1024;
const READ_CHUNK: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DuplicatePhase {
    #[default]
    Idle,
    Collecting,
    Comparing,
    Hashing,
    Finalizing,
    Complete,
    Partial,
    Cancelled,
    Failed,
}

/// Conservative is the default for unknown storage and rotational media.
/// Parallel is an explicit opt-in, not a drive-letter-based SSD guess.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DuplicateReadMode {
    #[default]
    Conservative,
    Parallel,
}

#[derive(Clone, Copy, Debug)]
struct ReadLimits {
    partial: usize,
    full: usize,
}

impl DuplicateReadMode {
    fn limits(self) -> ReadLimits {
        match self {
            Self::Conservative => ReadLimits {
                partial: 1,
                full: 1,
            },
            Self::Parallel => ReadLimits {
                partial: 4,
                full: 2,
            },
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DuplicateProgress {
    /// Completed candidate files, counted once across both passes.
    pub processed_files: u64,
    pub total_candidates: u64,
    /// Actual bytes read, including prefixes and failed/interrupted reads.
    pub hashed_bytes: u64,
    pub phase_processed: u64,
    pub phase_total: u64,
    pub partial_rejected: u64,
    pub full_candidates: u64,
    /// Only exposed for single-reader stages; parallel stages have no one path.
    pub current_idx: Option<u32>,
    pub inaccessible: u64,
    pub changed_during_read: u64,
    pub skipped_unavailable: u64,
    pub skipped_by_limit: u64,
}

#[derive(Clone, Debug)]
pub struct DuplicateMember {
    pub idx: u32,
    pub logical_size: u64,
    pub allocated_size: u64,
    /// Fingerprint captured during full-content confirmation, never guessed
    /// from the model's older size snapshot.
    pub(in crate::app::disk_analysis_duplicates) stamp: reader::FileStamp,
}

impl DuplicateMember {
    pub fn physical_identity(&self) -> FileIdentity {
        self.stamp.identity
    }
}

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub logical_size: u64,
    pub members: Vec<DuplicateMember>,
    pub recoverable: u64,
    pub has_hardlinks: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DuplicateStats {
    pub total_candidates: u64,
    pub processed_files: u64,
    pub hashed_bytes: u64,
    pub partial_rejected: u64,
    pub full_candidates: u64,
    pub inaccessible: u64,
    pub changed_during_read: u64,
    pub skipped_unavailable: u64,
    pub skipped_by_limit: u64,
    pub hardlink_only_groups: u64,
}

#[derive(Clone, Debug)]
pub struct DuplicateReport {
    pub groups: Vec<DuplicateGroup>,
    pub total_recoverable: u64,
    pub stats: DuplicateStats,
}

#[derive(Debug)]
pub(crate) enum DuplicateFinish {
    Complete(Box<DuplicateReport>),
    Partial(Box<DuplicateReport>),
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum DuplicateMessage {
    Progress(DuplicatePhase, DuplicateProgress),
    Finished(DuplicateFinish),
}

pub(crate) type PathResolver = Arc<dyn Fn(u32) -> Option<PathBuf> + Send + Sync>;
pub type FileIdentity = [u64; 2];

#[cfg(test)]
mod tests;
