//! Process-wide admission control: separate jobs cannot multiply a disk's budget.
use crate::profile::{Policy, Resource};
use crate::Progress;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

#[derive(Default)]
struct Count {
    total: usize,
    large: usize,
    limits: BTreeMap<(usize, usize), usize>,
}

#[derive(Default)]
pub(crate) struct Gate {
    counts: Mutex<BTreeMap<Resource, Count>>,
    wake: Condvar,
}

pub(crate) fn global() -> &'static Gate {
    static GATE: OnceLock<Gate> = OnceLock::new();
    GATE.get_or_init(Gate::default)
}

impl Gate {
    pub(crate) fn acquire<'a>(
        &'a self,
        policy: &'a Policy,
        large: bool,
        progress: &'a Progress,
    ) -> Option<Permit<'a>> {
        loop {
            // Never hold the disk-budget mutex while sleeping on pause.
            if progress.checkpoint().is_err() {
                return None;
            }
            let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            let available = policy.resources.iter().all(|key| {
                counts.get(key).is_none_or(|count| {
                    let total_limit = count
                        .limits
                        .keys()
                        .map(|limit| limit.0)
                        .min()
                        .unwrap_or(policy.info.workers)
                        .min(policy.info.workers);
                    let large_limit = count
                        .limits
                        .keys()
                        .map(|limit| limit.1)
                        .min()
                        .unwrap_or(policy.info.large_workers)
                        .min(policy.info.large_workers);
                    count.total < total_limit && count.large + usize::from(large) <= large_limit
                })
            });
            if available {
                for key in &policy.resources {
                    let count = counts.entry(*key).or_default();
                    count.total += 1;
                    count.large += usize::from(large);
                    *count
                        .limits
                        .entry((policy.info.workers, policy.info.large_workers))
                        .or_default() += 1;
                }
                let active = progress.telemetry.active.fetch_add(1, Ordering::Relaxed) + 1;
                progress
                    .telemetry
                    .peak_active
                    .fetch_max(active, Ordering::Relaxed);
                return Some(Permit {
                    gate: self,
                    policy,
                    large,
                    progress,
                    held: Cell::new(true),
                });
            }
            // Cancellation/pause has its own condvar. This bounded timed wait
            // reacts to those controls without busy-spinning or cross-locking.
            progress.telemetry.waiting.fetch_add(1, Ordering::Relaxed);
            let _waiting = Waiting(progress);
            let _ = self
                .wake
                .wait_timeout(counts, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

struct Waiting<'a>(&'a Progress);
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.telemetry.waiting.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) struct Permit<'a> {
    gate: &'a Gate,
    policy: &'a Policy,
    large: bool,
    progress: &'a Progress,
    held: Cell<bool>,
}

impl Permit<'_> {
    pub(crate) fn checkpoint(&self) -> std::io::Result<()> {
        // Release before acknowledging pause; other jobs must remain usable.
        self.progress.checkpoint_with(|| self.release())?;
        if !self.held.get() {
            let Some(replacement) = self.gate.acquire(self.policy, self.large, self.progress)
            else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "Copy cancelled",
                ));
            };
            replacement.held.set(false); // Transfer the acquired slot to this guard.
            self.held.set(true);
        }
        Ok(())
    }

    fn release(&self) {
        if !self.held.replace(false) {
            return;
        }
        let mut counts = self.gate.counts.lock().unwrap_or_else(|e| e.into_inner());
        for key in &self.policy.resources {
            if let Some(count) = counts.get_mut(key) {
                count.total -= 1;
                count.large -= usize::from(self.large);
                let limit = (self.policy.info.workers, self.policy.info.large_workers);
                if let Some(users) = count.limits.get_mut(&limit) {
                    *users -= 1;
                    if *users == 0 {
                        count.limits.remove(&limit);
                    }
                }
                if count.total == 0 {
                    counts.remove(key);
                }
            }
        }
        self.progress
            .telemetry
            .active
            .fetch_sub(1, Ordering::Relaxed);
        self.gate.wake.notify_all();
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests;
