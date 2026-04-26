use std::collections::BTreeMap;

use mtt_search_protocol::VolumeStatus;
use parking_lot::RwLock;

#[derive(Default)]
pub struct IndexingProgress {
    inner: RwLock<BTreeMap<char, VolumeStatus>>,
}

impl IndexingProgress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_scanning(&self, drive_letter: char, files_indexed: u64, phase: &str) {
        self.update(drive_letter, "scanning", files_indexed, phase, None, None);
    }

    pub fn update(
        &self,
        drive_letter: char,
        state: &str,
        files_indexed: u64,
        phase: &str,
        phase_progress: Option<u64>,
        phase_total: Option<u64>,
    ) {
        self.inner.write().insert(
            drive_letter,
            VolumeStatus {
                drive_letter,
                state: state.to_string(),
                files_indexed,
                phase: phase.to_string(),
                phase_progress,
                phase_total,
                sizes_loading: false,
            },
        );
    }

    pub fn set_error(&self, drive_letter: char, files_indexed: u64, phase: &str) {
        self.update(drive_letter, "error", files_indexed, phase, None, None);
    }

    pub fn clear(&self, drive_letter: char) {
        self.inner.write().remove(&drive_letter);
    }

    pub fn snapshot(&self) -> Vec<VolumeStatus> {
        self.inner.read().values().cloned().collect()
    }
}
