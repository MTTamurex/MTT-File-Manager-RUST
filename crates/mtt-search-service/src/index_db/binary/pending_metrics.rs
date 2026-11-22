//! Persist targeted metric retries without blocking the whole volume checkpoint.
//! Uses reserved v7 bits; older readers see incomplete metrics and fail closed
//! into their existing size refresh, rather than trusting untracked pending work.
use super::SEQUENCE_AWARE_METRICS;
use crate::file_index::{FileRecord, VolumeIndex};

const COMPLETE_METRICS: u64 = 1 << 2;
const PENDING_METRICS: u64 = 1 << 4;
const RECORD_PENDING_METRICS: u8 = 1;

pub(super) fn flags(index: &VolumeIndex) -> u64 {
    if !index.sizes_loaded {
        return 0;
    }
    SEQUENCE_AWARE_METRICS
        | if index.pending_size_refresh.is_empty() {
            COMPLETE_METRICS
        } else {
            PENDING_METRICS
        }
}

pub(super) fn mark_record(index: &VolumeIndex, frn: u64, record: &mut FileRecord) {
    record._pad = if index.sizes_loaded && index.pending_size_refresh.contains(&frn) {
        RECORD_PENDING_METRICS
    } else {
        0
    };
}

pub(super) fn restore(index: &mut VolumeIndex, flags: u64) {
    index.sizes_loaded =
        flags & SEQUENCE_AWARE_METRICS != 0 && flags & (COMPLETE_METRICS | PENDING_METRICS) != 0;
    if index.sizes_loaded && flags & PENDING_METRICS != 0 {
        index.pending_size_refresh.extend(
            index
                .records
                .iter()
                .filter(|(_, record)| record._pad & RECORD_PENDING_METRICS != 0)
                .map(|(&frn, _)| frn),
        );
    }
}

#[cfg(test)]
mod tests;
