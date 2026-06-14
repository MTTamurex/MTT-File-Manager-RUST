//! Types for thumbnail worker system
//!
//! This module contains the core data structures used by the thumbnail system.

use crate::infrastructure::io_priority::IOPriority;
use std::path::PathBuf;
use std::time::Instant;

/// Legacy alias for backwards compatibility with old ThumbnailPriority enum
/// High -> Interactive, Low -> Prefetch
pub type ThumbnailPriority = IOPriority;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailRequestSource {
    Normal,
    BulkScan,
}

/// Thumbnail request with priority and metadata
#[derive(Debug, Clone)]
pub struct ThumbnailRequest {
    pub path: PathBuf,
    pub generation: usize,
    pub size: u32,
    pub request_epoch: u64,
    pub priority: IOPriority,
    pub directory_index: Option<usize>,
    /// File modification time (seconds since UNIX_EPOCH) from folder enumeration.
    /// When > 0, avoids redundant std::fs::metadata() syscalls on HDD.
    /// When 0, falls back to reading metadata from disk.
    pub modified: u64,
    pub source: ThumbnailRequestSource,
    pub track_bulk_progress: bool,
    /// Original priority assigned by the bulk scan. When the same path is
    /// promoted to a visible request and later survives a navigation cleanup,
    /// this lets the queue restore the bulk work to its initial priority.
    pub bulk_priority: Option<IOPriority>,
    pub bulk_session: Option<u64>,
    pub queued_at: Instant,
}
