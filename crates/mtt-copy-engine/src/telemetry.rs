//! Numeric-only observations. Transfer bytes are API progress, not disk flushes.
use crate::PolicyInfo;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Preparing,
    Profiling,
    Transferring,
    Finished,
}

#[derive(Default)]
pub struct Telemetry {
    phase: AtomicUsize,
    policy: Mutex<Option<PolicyInfo>>,
    pub active: AtomicUsize,
    pub waiting: AtomicUsize,
    pub peak_active: AtomicUsize,
    /// Monotonic bytes reported by CopyFileEx, including failed attempts.
    pub transferred: AtomicU64,
}

impl Telemetry {
    pub fn phase(&self) -> Phase {
        match self.phase.load(Ordering::Relaxed) {
            1 => Phase::Profiling,
            2 => Phase::Transferring,
            3 => Phase::Finished,
            _ => Phase::Preparing,
        }
    }
    pub fn policy(&self) -> Option<PolicyInfo> {
        *self.policy.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn set_policy(&self, policy: PolicyInfo) {
        *self.policy.lock().unwrap_or_else(|e| e.into_inner()) = Some(policy);
    }
    pub(crate) fn set_phase(&self, phase: Phase) {
        self.phase.store(
            match phase {
                Phase::Preparing => 0,
                Phase::Profiling => 1,
                Phase::Transferring => 2,
                Phase::Finished => 3,
            },
            Ordering::Relaxed,
        );
    }
    pub(crate) fn run(&self) -> Run<'_> {
        Run(self)
    }
}

pub(crate) struct Run<'a>(&'a Telemetry);
impl Drop for Run<'_> {
    fn drop(&mut self) {
        self.0.set_phase(Phase::Finished);
    }
}
