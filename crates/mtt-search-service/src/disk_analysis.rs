//! Full-volume snapshot builder for disk usage analysis.
//!
//! Copies the live in-memory MFT/USN index of one NTFS volume into a
//! serializable snapshot. Performs no disk I/O; the only cost is the
//! O(n) copy under the per-volume read lock.

use crate::file_index::IndexState;
use crate::volume_indices::VolumeIndexHandle;
use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

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

        let mut records = Vec::with_capacity(vol.records.len());
        for (frn, record) in vol.records.iter() {
            records.push(DiskAnalysisRecord {
                frn: *frn,
                parent_frn: record.parent_ref,
                name: vol.names.get(record.name_ref()).to_string(),
                size: record.size,
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
