//! Bounded, coalesced folder metadata reads. Filesystem I/O runs only in a worker.
use crate::domain::file_tag::normalize_tag_path_key;
use eframe::egui;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

// Match the existing watcher mtime recheck delay for Windows lazy writes.
const DEBOUNCE: Duration = Duration::from_secs(2);
const MAX_PENDING: usize = 1024;
const BATCH_SIZE: usize = 16;

mod worker;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshReason {
    Missing,
    Watcher,
    Manual,
}

impl RefreshReason {
    fn priority(self) -> u8 {
        match self {
            Self::Manual => 0,
            Self::Missing => 1,
            Self::Watcher => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FolderDates {
    pub modified: u64,
    pub created: Option<u64>,
}

struct Pending {
    path: PathBuf,
    due: Instant,
    reason: RefreshReason,
}

struct ReadResult {
    path: PathBuf,
    dates: Option<FolderDates>,
}

#[derive(Default)]
pub(crate) struct FolderTimestampState {
    pending: HashMap<String, Pending>,
    in_flight: HashMap<String, bool>,
    worker: Option<mpsc::Receiver<Vec<ReadResult>>>,
}

impl FolderTimestampState {
    pub(crate) fn request(&mut self, path: &Path, reason: RefreshReason, now: Instant) -> bool {
        let key = normalize_tag_path_key(path);
        if reason == RefreshReason::Missing
            && (self.pending.contains_key(&key) || self.in_flight.contains_key(&key))
        {
            return false;
        }
        if let Some(valid) = self.in_flight.get_mut(&key) {
            *valid = false;
        }
        if let Some(pending) = self.pending.get_mut(&key) {
            if reason != RefreshReason::Watcher {
                pending.reason = reason;
                pending.due = now;
            } else if pending.reason == RefreshReason::Watcher {
                pending.due = now + DEBOUNCE;
            }
        } else {
            // A burst of background events must not drop the user's F5.
            // Keep the bound by replacing one lower-priority queued request.
            if self.pending.len() >= MAX_PENDING && reason == RefreshReason::Manual {
                let victim = self
                    .pending
                    .iter()
                    .filter(|(_, request)| request.reason != RefreshReason::Manual)
                    .max_by_key(|(_, request)| (request.reason.priority(), request.due))
                    .map(|(key, _)| key.clone());
                if let Some(key) = victim {
                    self.pending.remove(&key);
                }
            }
            if self.pending.len() >= MAX_PENDING {
                log::warn!("[FOLDER-DATES] Metadata refresh queue is full");
                return false;
            }
            self.pending.insert(
                key,
                Pending {
                    path: path.to_path_buf(),
                    due: if reason == RefreshReason::Watcher {
                        now + DEBOUNCE
                    } else {
                        now
                    },
                    reason,
                },
            );
        }
        true
    }

    fn take_due(&mut self, now: Instant) -> Vec<PathBuf> {
        if !self.in_flight.is_empty() {
            return Vec::new();
        }
        let mut due: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, request)| request.due <= now)
            .map(|(key, request)| (key.clone(), (request.reason.priority(), request.due)))
            .collect();
        due.sort_unstable_by_key(|(_, deadline)| *deadline);
        due.into_iter()
            .take(BATCH_SIZE)
            .map(|(key, _)| {
                let pending = self.pending.remove(&key).expect("selected pending request");
                self.in_flight.insert(key, true);
                pending.path
            })
            .collect()
    }

    fn finish(&mut self, results: Vec<ReadResult>) -> Vec<(PathBuf, FolderDates)> {
        let in_flight = std::mem::take(&mut self.in_flight);
        results
            .into_iter()
            .filter_map(|result| {
                // A watcher event or F5 received during this read supersedes it.
                let key = normalize_tag_path_key(&result.path);
                if in_flight.get(&key) != Some(&true) || self.pending.contains_key(&key) {
                    None
                } else {
                    result.dates.map(|dates| (result.path, dates))
                }
            })
            .collect()
    }

    pub(crate) fn poll(
        &mut self,
        ctx: &egui::Context,
        now: Instant,
    ) -> Vec<(PathBuf, FolderDates)> {
        let mut ready = Vec::new();
        if let Some(receiver) = &self.worker {
            match receiver.try_recv() {
                Ok(results) => {
                    self.worker = None;
                    ready = self.finish(results);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.in_flight.clear();
                    log::warn!("[FOLDER-DATES] Metadata worker stopped unexpectedly");
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let paths = self.take_due(now);
        if !paths.is_empty() {
            let retry = paths.clone();
            let (tx, rx) = mpsc::channel();
            let repaint = ctx.clone();
            match std::thread::Builder::new()
                .name("folder-meta-resolve".into())
                .spawn(move || {
                    worker::run(paths, tx, repaint);
                }) {
                Ok(_) => self.worker = Some(rx),
                Err(_) => {
                    self.in_flight.clear();
                    for path in retry {
                        self.request(&path, RefreshReason::Watcher, now);
                    }
                    log::warn!("[FOLDER-DATES] Could not start metadata worker");
                }
            }
        }
        if let Some(wait) = self.next_wakeup(now) {
            ctx.request_repaint_after(wait);
        }
        ready
    }

    fn next_wakeup(&self, now: Instant) -> Option<Duration> {
        if self.worker.is_some() {
            // Normally the worker wakes the UI itself, including when unwinding.
            None
        } else {
            self.pending
                .values()
                .map(|p| p.due.saturating_duration_since(now))
                .min()
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod review_tests;
