//! One bounded snapshot, not a queue of per-chunk notifications.
use std::sync::Mutex;
use std::time::{Duration, Instant};

const UPDATE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeleteStage {
    #[default]
    Pair,
    Metadata,
    TargetContent,
    SurvivorContent,
    Revalidate,
    Windows,
}

impl DeleteStage {
    pub const ALL: [Self; 6] = [
        Self::Pair,
        Self::Metadata,
        Self::TargetContent,
        Self::SurvivorContent,
        Self::Revalidate,
        Self::Windows,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteFailure {
    InvalidPair,
    TargetUnavailableOrChanged,
    SurvivorUnavailableOrChanged,
    TargetReadFailed,
    SurvivorReadFailed,
    ContentMismatch,
    PathsChanged,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeleteProgress {
    pub stage: DeleteStage,
    pub bytes_read: u64,
    pub total_bytes: u64,
    pub failure: Option<DeleteFailure>,
}

impl DeleteProgress {
    pub fn fraction(&self) -> Option<f32> {
        matches!(
            self.stage,
            DeleteStage::TargetContent | DeleteStage::SurvivorContent
        )
        .then(|| {
            if self.total_bytes == 0 {
                1.0
            } else {
                (self.bytes_read as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32
            }
        })
    }
}

pub(super) fn snapshot(shared: &Mutex<DeleteProgress>) -> DeleteProgress {
    *shared
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) struct Reporter<'a> {
    shared: &'a Mutex<DeleteProgress>,
    last_update: Instant,
    total: u64,
}

impl<'a> Reporter<'a> {
    pub fn new(shared: &'a Mutex<DeleteProgress>) -> Self {
        Self {
            shared,
            last_update: Instant::now(),
            total: 0,
        }
    }

    pub fn begin(&mut self, stage: DeleteStage, total: u64) {
        *self.shared.lock().unwrap_or_else(|p| p.into_inner()) = DeleteProgress {
            stage,
            total_bytes: total,
            ..Default::default()
        };
        self.total = total;
        self.last_update = Instant::now();
    }

    pub fn read(&mut self, bytes: u64) {
        let now = Instant::now();
        if bytes >= self.total || now.duration_since(self.last_update) >= UPDATE_INTERVAL {
            self.shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .bytes_read = bytes.min(self.total);
            self.last_update = now;
        }
    }

    pub fn fail(&self, failure: DeleteFailure) {
        self.shared
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .failure = Some(failure);
    }
}

#[cfg(test)]
mod tests;
