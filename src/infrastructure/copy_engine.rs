//! In-memory copy operations, retained until the user dismisses their results.
pub(crate) mod diagnostics;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeKind {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub kind: OutcomeKind,
    pub message: Option<String>,
}

pub struct CopyOperation {
    pub sources: Vec<PathBuf>,
    pub destination: PathBuf,
    pub progress: mtt_copy_engine::Progress,
    outcome: Mutex<Option<(Outcome, Duration)>>,
}

impl CopyOperation {
    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(outcome, _)| outcome.clone())
    }

    /// Includes preparation and I/O waits, but excludes user-requested pauses.
    /// Terminal durations are retained independently of panel visibility.
    pub fn elapsed(&self) -> Duration {
        let outcome = self.outcome.lock().unwrap_or_else(|e| e.into_inner());
        outcome
            .as_ref()
            .map(|(_, elapsed)| *elapsed)
            .unwrap_or_else(|| self.progress.active_elapsed())
    }
}

#[derive(Default)]
pub struct CopyRuntime {
    pub enabled: AtomicBool,
    next_id: AtomicU64,
    jobs: Mutex<BTreeMap<u64, Arc<CopyOperation>>>,
}

impl CopyRuntime {
    pub fn start(&self, sources: Vec<PathBuf>, destination: PathBuf) -> CopyJob {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let operation = Arc::new(CopyOperation {
            sources,
            destination,
            progress: Default::default(),
            outcome: Mutex::new(None),
        });
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, operation.clone());
        CopyJob { operation }
    }

    pub fn jobs(&self) -> Vec<(u64, Arc<CopyOperation>)> {
        self.jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(&id, operation)| (id, operation.clone()))
            .collect()
    }

    /// Only terminal results can be dismissed. Hiding a running job must never
    /// make its pause/cancel controls unreachable.
    pub fn dismiss(&self, id: u64) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.get(&id).is_some_and(|job| job.outcome().is_some()) {
            jobs.remove(&id);
        }
    }
}

pub struct CopyJob {
    pub operation: Arc<CopyOperation>,
}

impl CopyJob {
    pub fn finish(&self, outcome: Outcome) {
        *self
            .operation
            .outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner()) =
            Some((outcome, self.operation.progress.active_elapsed()));
    }
}

impl Drop for CopyJob {
    fn drop(&mut self) {
        let mut outcome = self
            .operation
            .outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if outcome.is_none() {
            *outcome = Some((
                Outcome {
                    kind: OutcomeKind::Failed,
                    message: Some(rust_i18n::t!("copy_engine.interrupted").to_string()),
                },
                self.operation.progress.active_elapsed(),
            ));
        }
    }
}

#[cfg(test)]
mod tests;
