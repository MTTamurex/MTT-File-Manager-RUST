//! Per-volume locking for the shared index store.
//!
//! Phase 5 / F5.4 — replaces the old `Arc<RwLock<Vec<VolumeIndex>>>`
//! (one big lock guarding all volumes) with `Arc<RwLock<Vec<Arc<RwLock<VolumeIndex>>>>>`.
//!
//! - The **outer** lock guards only the *membership* of the vec (rare:
//!   adding a newly-discovered drive letter or replacing a re-indexed volume).
//! - The **inner** per-volume lock guards mutations and reads of one
//!   `VolumeIndex`.
//!
//! This means a long USN write on `D:\` no longer blocks a search reader on
//! `C:\` — they take independent inner locks.

use parking_lot::RwLock;
use std::sync::Arc;

use crate::file_index::VolumeIndex;

/// Cheap, cloneable handle to a single volume's index.
pub type VolumeIndexHandle = Arc<RwLock<VolumeIndex>>;

/// Shared registry of all known volume indices.
pub type SharedVolumeIndices = Arc<RwLock<Vec<VolumeIndexHandle>>>;

/// Create a new empty shared registry.
pub fn new_shared() -> SharedVolumeIndices {
    Arc::new(RwLock::new(Vec::new()))
}

/// Wrap a `VolumeIndex` into a handle.
#[cfg_attr(not(test), allow(dead_code))]
pub fn handle_from(index: VolumeIndex) -> VolumeIndexHandle {
    Arc::new(RwLock::new(index))
}

/// Insert or replace the volume entry for `new_index.drive_letter`.
///
/// If a handle for that drive letter already exists, its inner contents are
/// overwritten in place (preserving the `Arc` so any outstanding handles
/// stay valid). Otherwise a new handle is appended. Returns the live handle
/// either way so the caller can keep mutating without re-acquiring the
/// outer lock.
pub fn upsert(indices: &SharedVolumeIndices, new_index: VolumeIndex) -> VolumeIndexHandle {
    let drive_letter = new_index.drive_letter;
    let mut outer = indices.write();
    if let Some(existing) = outer
        .iter()
        .find(|h| h.read().drive_letter == drive_letter)
        .cloned()
    {
        *existing.write() = new_index;
        existing
    } else {
        let handle = Arc::new(RwLock::new(new_index));
        outer.push(handle.clone());
        handle
    }
}

/// Look up the handle for `drive_letter`. The outer read lock is released
/// before returning, so the caller can take the inner lock without holding
/// any other lock.
pub fn find_handle(indices: &SharedVolumeIndices, drive_letter: char) -> Option<VolumeIndexHandle> {
    indices
        .read()
        .iter()
        .find(|h| h.read().drive_letter == drive_letter)
        .cloned()
}

/// Snapshot all handles (clones the `Arc`s). The outer read lock is held only
/// for the duration of the clone.
pub fn snapshot_handles(indices: &SharedVolumeIndices) -> Vec<VolumeIndexHandle> {
    indices.read().iter().cloned().collect()
}
