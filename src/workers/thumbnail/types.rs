//! Types for thumbnail worker system
//!
//! This module contains the core data structures used by the thumbnail system.

use crate::infrastructure::io_priority::IOPriority;
use std::path::PathBuf;

/// Legacy alias for backwards compatibility with old ThumbnailPriority enum
/// High -> Interactive, Low -> Prefetch
pub type ThumbnailPriority = IOPriority;

/// Thumbnail request with priority and metadata
#[derive(Debug, Clone)]
pub struct ThumbnailRequest {
    pub path: PathBuf,
    pub generation: usize,
    pub size: u32,
    pub priority: IOPriority,
    pub directory_index: Option<usize>,
    /// File modification time (seconds since UNIX_EPOCH) from folder enumeration.
    /// When > 0, avoids redundant std::fs::metadata() syscalls on HDD.
    /// When 0, falls back to reading metadata from disk.
    pub modified: u64,
}
