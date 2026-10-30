use super::{FileMetrics, VolumeIndex};
use std::time::Instant;

#[cfg(test)]
mod tests;

impl VolumeIndex {
    /// Apply a measured value, including shrink/zero, and invalidate ancestor
    /// folder caches only after the corrected value is visible in the index.
    pub(crate) fn update_file_metrics(
        &mut self,
        frn: u64,
        metrics: FileMetrics,
        now: Instant,
    ) -> bool {
        let Some(record) = self.records.get_mut(&frn) else {
            return false;
        };
        if record.is_dir()
            || (record.size, record.allocated_size)
                == (metrics.logical_size, metrics.allocated_size)
        {
            return false;
        }
        (record.size, record.allocated_size) = (metrics.logical_size, metrics.allocated_size);
        let mut parent = Self::normalized_parent_bucket(frn, record.parent_ref);
        self.binary_dirty = true;
        // A shared batch timestamp also avoids rewalking common ancestors and
        // terminates malformed cycles without allocating a per-file visited set.
        for _ in 0..self.records.len() {
            if self.dir_modified_at.get(&parent) == Some(&now) {
                break;
            }
            self.dir_modified_at.insert(parent, now);
            let Some(directory) = self.records.get(&parent) else {
                break;
            };
            if !directory.is_dir() || directory.parent_ref == parent {
                break;
            }
            parent = Self::normalized_parent_bucket(parent, directory.parent_ref);
        }
        true
    }
}
