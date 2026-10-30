//! Bounded, per-stage readers; the coordinator alone owns progress/results.

use super::reader::{ReadJob, ReadOutcome, ValidatedHash};
use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Sender};
use std::time::{Duration, Instant};

enum Event {
    Started(u32),
    Bytes(u64),
    Finished(u32, ReadOutcome),
    Failed,
}

pub(super) fn publish(
    tx: &Sender<DuplicateMessage>,
    phase: DuplicatePhase,
    progress: &DuplicateProgress,
) {
    let _ = tx.send(DuplicateMessage::Progress(phase, progress.clone()));
}

pub(super) fn run_stage(
    jobs: (usize, impl Fn(usize) -> ReadJob + Sync),
    slots: usize,
    phase: DuplicatePhase,
    cancel: &AtomicBool,
    progress: &mut DuplicateProgress,
    tx: &Sender<DuplicateMessage>,
    read: impl Fn(ReadJob, &mut dyn FnMut(u64)) -> ReadOutcome + Sync,
) -> Result<Option<HashMap<u32, ValidatedHash>>, String> {
    let (count, job_at) = jobs;
    progress.phase_processed = 0;
    progress.phase_total = count as u64;
    progress.current_idx = None;
    publish(tx, phase, progress);
    if count == 0 {
        return Ok(Some(HashMap::new()));
    }
    let slots = slots.clamp(1, 4).min(count);
    let next = AtomicUsize::new(0);
    let mut hashes = HashMap::new();
    std::thread::scope(|scope| -> Result<(), String> {
        let (events, receiver) = sync_channel(slots * 2);
        for slot in 0..slots {
            let events = events.clone();
            let next = &next;
            let read = &read;
            let job_at = &job_at;
            std::thread::Builder::new()
                .name(format!("duplicate-reader-{slot}"))
                .spawn_scoped(scope, move || {
                    while !cancel.load(Ordering::Acquire) {
                        let position = next.fetch_add(1, Ordering::Relaxed);
                        if position >= count {
                            break;
                        }
                        let job = job_at(position);
                        if slots == 1 && events.send(Event::Started(job.idx)).is_err() {
                            break;
                        }
                        let mut pending = 0u64;
                        let outcome =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                read(job, &mut |bytes| {
                                    pending = pending.saturating_add(bytes);
                                    if pending >= 16 * 1024 * 1024 {
                                        if events.send(Event::Bytes(pending)).is_err() {
                                            cancel.store(true, Ordering::Release);
                                        }
                                        pending = 0;
                                    }
                                })
                            }));
                        let outcome = match outcome {
                            Ok(outcome) => outcome,
                            Err(_) => {
                                cancel.store(true, Ordering::Release);
                                let _ = events.send(Event::Failed);
                                break;
                            }
                        };
                        if pending > 0 && events.send(Event::Bytes(pending)).is_err() {
                            break;
                        }
                        if events.send(Event::Finished(job.idx, outcome)).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|_| {
                    cancel.store(true, Ordering::Release);
                    "cannot start duplicate reader".to_string()
                })?;
        }
        drop(events);
        let mut last_update = Instant::now();
        let mut failed = false;
        for event in receiver {
            match event {
                Event::Failed => failed = true,
                Event::Started(idx) => progress.current_idx = Some(idx),
                Event::Bytes(bytes) => {
                    progress.hashed_bytes = progress.hashed_bytes.saturating_add(bytes)
                }
                Event::Finished(idx, outcome) => {
                    progress.phase_processed += 1;
                    let success = matches!(&outcome, ReadOutcome::Hashed(_));
                    match outcome {
                        ReadOutcome::Hashed(hash) => {
                            hashes.insert(idx, hash);
                        }
                        ReadOutcome::Inaccessible => progress.inaccessible += 1,
                        ReadOutcome::Unavailable => progress.skipped_unavailable += 1,
                        ReadOutcome::Changed => progress.changed_during_read += 1,
                        ReadOutcome::Cancelled => {}
                    }
                    // Successful prefixes have not finished the overall job yet.
                    if phase == DuplicatePhase::Hashing || !success {
                        progress.processed_files += 1;
                    }
                }
            }
            if last_update.elapsed() >= Duration::from_millis(100) {
                publish(tx, phase, progress);
                last_update = Instant::now();
            }
        }
        if failed {
            Err("duplicate reader panicked".into())
        } else {
            Ok(())
        }
    })?;
    progress.current_idx = None;
    publish(tx, phase, progress);
    Ok((!cancel.load(Ordering::Acquire)).then_some(hashes))
}

#[cfg(test)]
mod tests;
