//! Full-volume snapshot builder for disk usage analysis.
//!
//! Copies the live in-memory MFT/USN index of one NTFS volume into a
//! serializable snapshot. Performs no disk I/O; the only cost is the
//! O(n) copy under the per-volume read lock.

use crate::file_index::IndexState;
use crate::volume_indices::VolumeIndexHandle;
use mtt_search_protocol::{
    validate_disk_analysis_record_count, DiskAnalysisRecord, DiskAnalysisSnapshot,
    DISK_ANALYSIS_BUSY_ERROR, DISK_ANALYSIS_REFRESHING_ERROR, MAX_DISK_ANALYSIS_PAYLOAD_BYTES,
};
use std::sync::atomic::{AtomicBool, Ordering};

static SNAPSHOT_ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct SnapshotPermit;

impl Drop for SnapshotPermit {
    fn drop(&mut self) {
        SNAPSHOT_ACTIVE.store(false, Ordering::Release);
    }
}

pub fn try_acquire_snapshot() -> Result<SnapshotPermit, String> {
    SNAPSHOT_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| SnapshotPermit)
        .map_err(|_| DISK_ANALYSIS_BUSY_ERROR.to_string())
}

/// Copy every live record of the volume into a client-facing snapshot.
///
/// The read lock is held only for the duration of the copy; serialization
/// happens afterwards on the caller side so the USN writer is not blocked
/// while the payload is encoded and written to the pipe.
pub fn build_snapshot(handle: &VolumeIndexHandle) -> Result<DiskAnalysisSnapshot, String> {
    let (drive_letter, records) = {
        let vol = handle.read();
        if !matches!(vol.state, IndexState::Ready) {
            return Err("Volume not ready".to_string());
        }
        if !vol.sizes_loaded {
            return Err("Sizes not loaded".to_string());
        }
        // Queued FRNs may remain pending indefinitely when an open-by-ID probe
        // is temporarily unavailable. They do not make the read-locked index
        // unsafe to snapshot. Only block while a drained batch is actively
        // being read and has not yet been applied.
        if vol.size_refresh_in_progress {
            return Err(DISK_ANALYSIS_REFRESHING_ERROR.to_string());
        }
        validate_disk_analysis_record_count(vol.records.len())?;

        // Bincode's fixed fields plus enum/vector framing fit comfortably in
        // this conservative per-record allowance. Reject before cloning names.
        let mut estimated_payload = 64usize;
        for (_, record) in vol.records.iter() {
            estimated_payload = estimated_payload
                .checked_add(64)
                .and_then(|size| size.checked_add(vol.names.get(record.name_ref()).len()))
                .ok_or_else(|| "Disk analysis payload size overflow".to_string())?;
            if estimated_payload > MAX_DISK_ANALYSIS_PAYLOAD_BYTES {
                return Err(format!(
                    "Disk analysis payload too large (estimated {} bytes, max {})",
                    estimated_payload, MAX_DISK_ANALYSIS_PAYLOAD_BYTES
                ));
            }
        }

        let mut records = Vec::with_capacity(vol.records.len());
        for (frn, record) in vol.records.iter() {
            records.push(DiskAnalysisRecord {
                frn: *frn,
                parent_frn: record.parent_ref,
                name: vol.names.get(record.name_ref()).to_string(),
                size: record.size,
                allocated_size: record.allocated_size,
                is_dir: record.is_dir(),
                is_reparse: vol.reparse_points.contains(frn),
            });
        }
        (vol.drive_letter, records)
    };

    Ok(DiskAnalysisSnapshot {
        drive_letter,
        records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::{IndexState, VolumeIndex};

    fn ready_index() -> VolumeIndexHandle {
        let mut index = VolumeIndex::empty('C');
        assert!(index.insert_record(5, "", 5, true, false));
        assert!(index.insert_record(10, "sparse.bin", 5, false, false));
        let record = index.records.get_mut(&10).unwrap();
        record.size = 1 << 40;
        record.allocated_size = 8_192;
        index.state = IndexState::Ready;
        index.sizes_loaded = true;
        crate::volume_indices::handle_from(index)
    }

    #[test]
    fn snapshot_preserves_logical_and_allocated_sizes() {
        let snapshot = build_snapshot(&ready_index()).unwrap();
        let record = snapshot
            .records
            .iter()
            .find(|record| record.frn == 10)
            .unwrap();
        assert_eq!(record.size, 1 << 40);
        assert_eq!(record.allocated_size, 8_192);
    }

    #[test]
    fn snapshot_allows_queued_size_refresh() {
        let handle = ready_index();
        handle.write().pending_size_refresh.insert(10);
        assert!(build_snapshot(&handle).is_ok());
    }

    #[test]
    fn snapshot_rejects_size_refresh_in_progress() {
        let handle = ready_index();
        handle.write().size_refresh_in_progress = true;
        assert!(build_snapshot(&handle)
            .unwrap_err()
            .contains("being refreshed"));
    }

    #[test]
    fn snapshot_permit_is_non_blocking_and_released_on_drop() {
        let first = try_acquire_snapshot().unwrap();
        assert!(try_acquire_snapshot().is_err());
        drop(first);
        assert!(try_acquire_snapshot().is_ok());
    }
}
