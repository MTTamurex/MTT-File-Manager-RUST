use super::reader::{read_candidate, ReadJob, ValidatedHash};
use super::workers::{publish, run_stage};
use super::*;
use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc::Sender, Mutex, OnceLock, TryLockError};
use std::time::Duration;

static VOLUME_SCAN_LOCKS: OnceLock<[Mutex<()>; 26]> = OnceLock::new();

pub(super) fn run_scan(
    model: Arc<DiskAnalysisModel>,
    root: u32,
    minimum: u64,
    resolver: PathResolver,
    cancel: Arc<AtomicBool>,
    tx: Sender<DuplicateMessage>,
    limits: ReadLimits,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        scan(&model, root, minimum, &resolver, &cancel, &tx, limits)
    }))
    .unwrap_or_else(|_| Err("duplicate worker panicked".into()));
    let finish = match result {
        Err(error) => DuplicateFinish::Failed(error),
        Ok(None) => DuplicateFinish::Cancelled,
        Ok(Some(report)) => {
            let stats = report.stats;
            if stats.inaccessible > 0
                || stats.changed_during_read > 0
                || stats.skipped_unavailable > 0
            {
                DuplicateFinish::Partial(Box::new(report))
            } else {
                DuplicateFinish::Complete(Box::new(report))
            }
        }
    };
    let _ = tx.send(DuplicateMessage::Finished(finish));
}

fn scan(
    model: &DiskAnalysisModel,
    root: u32,
    minimum: u64,
    resolver: &PathResolver,
    cancel: &AtomicBool,
    tx: &Sender<DuplicateMessage>,
    limits: ReadLimits,
) -> Result<Option<DuplicateReport>, String> {
    if root as usize >= model.nodes.len() {
        return Err("duplicate scan root outside captured model".into());
    }
    let cancelled = || cancel.load(Ordering::Acquire);
    let Some((groups, skipped)) = candidates::collect(model, root, minimum, cancelled) else {
        return Ok(None);
    };
    let mut progress = DuplicateProgress {
        total_candidates: groups.iter().map(|g| g.1.len() as u64).sum(),
        skipped_by_limit: skipped,
        ..Default::default()
    };
    if groups.is_empty() {
        return Ok(report::build(
            model,
            &groups,
            &HashMap::new(),
            &progress,
            cancelled,
        ));
    }

    // Overlapping sessions share a drive budget. Waiting remains cancellable.
    let locks = VOLUME_SCAN_LOCKS.get_or_init(|| std::array::from_fn(|_| Mutex::new(())));
    let drive = model.drive_letter.to_ascii_uppercase();
    if !drive.is_ascii_uppercase() {
        return Err("invalid duplicate scan drive".into());
    }
    let lock = &locks[(drive as u8 - b'A') as usize];
    let _guard = loop {
        if cancelled() {
            return Ok(None);
        }
        match lock.try_lock() {
            Ok(guard) => break guard,
            Err(TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(TryLockError::WouldBlock) => std::thread::park_timeout(Duration::from_millis(10)),
        }
    };

    // Keep bare indices, not a full metadata stamp per prefix job.
    let prefix_jobs: Vec<_> = groups
        .iter()
        .flat_map(|(_, members)| members.iter().copied())
        .collect();
    let Some(mut prefixes) = run_stage(
        (prefix_jobs.len(), |position| {
            let idx = prefix_jobs[position];
            ReadJob {
                idx,
                size: model.nodes[idx as usize].size,
                previous: None,
            }
        }),
        limits.partial,
        DuplicatePhase::Comparing,
        cancel,
        &mut progress,
        tx,
        |job, on_read| read_candidate(resolver, job, Some(PARTIAL_BYTES), cancel, on_read),
    )?
    else {
        return Ok(None);
    };
    drop(prefix_jobs);
    let mut confirmed: HashMap<u32, ValidatedHash> = HashMap::new();
    let mut full_jobs = Vec::new();
    for (size, members) in &groups {
        if cancelled() {
            return Ok(None);
        }
        if *size <= PARTIAL_BYTES {
            // Already complete, validated hashes; report grouping needs no
            // second copy of each digest or another read.
            for &idx in members {
                if cancelled() {
                    return Ok(None);
                }
                if let Some(hash) = prefixes.remove(&idx) {
                    confirmed.insert(idx, hash);
                    progress.processed_files += 1;
                }
            }
            continue;
        }
        let mut by_prefix: HashMap<[u8; 32], Vec<u32>> = HashMap::new();
        for &idx in members {
            if cancelled() {
                return Ok(None);
            }
            if let Some(hash) = prefixes.get(&idx) {
                // Keep bare indices in buckets, not growable vectors of full
                // stamps/digests for every unique prefix.
                by_prefix.entry(hash.digest).or_default().push(idx);
            }
        }
        for bucket in by_prefix.into_values() {
            if cancelled() {
                return Ok(None);
            }
            if bucket.len() == 1 {
                progress.partial_rejected += 1;
                progress.processed_files += 1;
                prefixes.remove(&bucket[0]);
            } else {
                for idx in bucket {
                    if let Some(hash) = prefixes.remove(&idx) {
                        full_jobs.push((idx, hash.stamp));
                    }
                }
            }
        }
    }
    drop(prefixes);
    progress.full_candidates = full_jobs.len() as u64;
    let Some(full) = run_stage(
        (full_jobs.len(), |position| {
            let (idx, stamp) = full_jobs[position];
            ReadJob {
                idx,
                size: stamp.size,
                previous: Some(stamp),
            }
        }),
        limits.full,
        DuplicatePhase::Hashing,
        cancel,
        &mut progress,
        tx,
        |job, on_read| read_candidate(resolver, job, None, cancel, on_read),
    )?
    else {
        return Ok(None);
    };
    confirmed.extend(full);
    publish(tx, DuplicatePhase::Finalizing, &progress);
    Ok(report::build(
        model, &groups, &confirmed, &progress, cancelled,
    ))
}
