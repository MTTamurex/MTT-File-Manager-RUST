use serde::{Deserialize, Serialize};

/// Maximum records accepted in a DiskAnalysis snapshot response.
/// Prevents a compromised or buggy service from flooding the client with
/// tens of millions of entries (4M records ~= worst-case ~200 MB encoded).
pub const MAX_DISK_ANALYSIS_RECORDS: usize = 4_000_000;
pub const MAX_DISK_ANALYSIS_PAYLOAD_BYTES: usize = 512 * 1024 * 1024;
pub const DISK_ANALYSIS_BUSY_ERROR: &str = "Another disk analysis is already in progress";
pub const DISK_ANALYSIS_REFRESHING_ERROR: &str = "File sizes are being refreshed; try again later";

pub fn validate_disk_analysis_record_count(count: usize) -> Result<(), String> {
    if count > MAX_DISK_ANALYSIS_RECORDS {
        return Err(format!(
            "too many disk analysis records ({count}, max {MAX_DISK_ANALYSIS_RECORDS})"
        ));
    }
    Ok(())
}

/// One filesystem node of a volume snapshot, keyed by File Reference Number.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiskAnalysisRecord {
    /// File Reference Number of this node.
    pub frn: u64,
    /// FRN of the parent directory (volume root references itself).
    pub parent_frn: u64,
    /// File name (UTF-8).
    pub name: String,
    /// File size in bytes (0 for directories).
    pub size: u64,
    /// Physical bytes allocated to all file data streams. Resident and fully
    /// sparse streams contribute zero; directories are zero.
    pub allocated_size: u64,
    /// Directory flag.
    pub is_dir: bool,
    /// Reparse point (junction/symlink); must not be descended into.
    pub is_reparse: bool,
}

/// Full-volume snapshot for disk usage analysis, copied from the service's
/// in-memory MFT/USN index. NTFS volumes only.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DiskAnalysisSnapshot {
    pub drive_letter: char,
    pub records: Vec<DiskAnalysisRecord>,
}

impl DiskAnalysisSnapshot {
    /// Validate deserialized snapshot bounds (record count cap).
    pub fn validate(&self) -> Result<(), String> {
        if !self.drive_letter.is_ascii_alphabetic() {
            return Err("disk analysis drive letter must be an ASCII letter".to_string());
        }
        validate_disk_analysis_record_count(self.records.len())
    }
}
