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

/// Remove a cached index only if the drive letter still refers to the same
/// volume. This prevents a delayed removal notification from deleting a new
/// volume that has already reused the letter.
pub fn remove_volume_if_identity_matches(
    indices: &SharedVolumeIndices,
    drive_letter: char,
    volume_id: &str,
) -> bool {
    let mut registry = indices.write();
    let Some(position) = registry.iter().position(|handle| {
        let volume = handle.read();
        volume.drive_letter == drive_letter && volume_ids_match(&volume.volume_id, volume_id)
    }) else {
        return false;
    };

    registry.remove(position);
    true
}

fn normalize_volume_id(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

fn volume_ids_match(left: &str, right: &str) -> bool {
    let left = normalize_volume_id(left);
    let right = normalize_volume_id(right);
    if left == right {
        return true;
    }

    let guid_token = |value: &str| {
        let lower = value.to_ascii_lowercase();
        let start = lower.find("volume{")?;
        let rest = &value[start..];
        let end = rest.find('}')? + 1;
        Some(rest[..end].to_ascii_lowercase())
    };
    guid_token(&left).is_some() && guid_token(&left) == guid_token(&right)
}

#[cfg(test)]
mod tests {
    use super::{new_shared, remove_volume_if_identity_matches, upsert};
    use crate::file_index::VolumeIndex;

    #[test]
    fn stale_removal_notification_cannot_remove_a_reused_drive_letter() {
        let indices = new_shared();
        let mut index = VolumeIndex::empty('R');
        index.volume_id = r"\\?\Volume{abcdef01-2345-6789-abcd-ef0123456789}\".to_string();
        upsert(&indices, index);

        assert!(!remove_volume_if_identity_matches(
            &indices,
            'R',
            r"\\?\Volume{fedcba98-7654-3210-fedc-ba9876543210}\"
        ));
        assert!(remove_volume_if_identity_matches(
            &indices,
            'R',
            r"\??\Volume{ABCDEF01-2345-6789-ABCD-EF0123456789}"
        ));
    }
}
