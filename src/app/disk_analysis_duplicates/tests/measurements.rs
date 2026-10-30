//! Explicit synthetic warm-cache experiment; not an SSD/HD benchmark.
use super::*;
use crate::app::disk_analysis_duplicates::reader::{read_candidate, ReadJob, ReadOutcome};
use std::collections::HashMap;
use std::time::Instant;

fn baseline(model: &DiskAnalysisModel, resolver: &PathResolver) -> DuplicateReport {
    let (groups, _) = candidates::collect(model, model.root, 1, || false).unwrap();
    let cancel = AtomicBool::new(false);
    let mut hashes = HashMap::new();
    let mut progress = DuplicateProgress::default();
    for (size, members) in &groups {
        for &idx in members {
            let job = ReadJob {
                idx,
                size: *size,
                previous: None,
            };
            let ReadOutcome::Hashed(hash) =
                read_candidate(resolver, job, None, &cancel, |n| progress.hashed_bytes += n)
            else {
                panic!("baseline read failed")
            };
            hashes.insert(idx, hash);
            progress.total_candidates += 1;
            progress.processed_files += 1;
        }
    }
    report::build(model, &groups, &hashes, &progress, || false).unwrap()
}

fn groups(report: &DuplicateReport) -> Vec<Vec<u32>> {
    let mut result: Vec<_> = report
        .groups
        .iter()
        .map(|g| g.members.iter().map(|m| m.idx).collect())
        .collect();
    result.sort();
    result
}

#[test]
fn optimized_results_match_full_read_reference_with_high_and_low_prefix_rejection() {
    for shared_prefix in [false, true] {
        let contents: Vec<_> = (0..8)
            .map(|i| {
                let mut bytes = vec![if shared_prefix { 7 } else { i }; PARTIAL_BYTES as usize * 2];
                let end = bytes.len() - 1;
                bytes[end] = i;
                bytes
            })
            .collect();
        let (_dir, model, resolver) = fixture(&contents);
        let reference = baseline(&model, &resolver);
        for mode in [DuplicateReadMode::Conservative, DuplicateReadMode::Parallel] {
            let (actual, _) = run(model.clone(), resolver.clone(), mode);
            assert_eq!(groups(&actual), groups(&reference));
            assert_eq!(actual.total_recoverable, reference.total_recoverable);
            if shared_prefix {
                assert_eq!(
                    actual.stats.hashed_bytes,
                    reference.stats.hashed_bytes + 8 * PARTIAL_BYTES
                );
            } else {
                assert_eq!(actual.stats.hashed_bytes, 8 * PARTIAL_BYTES);
            }
        }
    }
}

#[test]
#[ignore = "explicit synthetic I/O measurement; run with --ignored --nocapture"]
fn duplicate_scan_synthetic_measurements() {
    const SIZE: usize = 4 * 1024 * 1024;
    for shared_prefix in [false, true] {
        let contents: Vec<_> = (0..12u8)
            .map(|i| {
                let mut bytes = vec![
                    if shared_prefix {
                        7
                    } else {
                        i.saturating_sub(1)
                    };
                    SIZE
                ];
                let end = bytes.len() - 1;
                bytes[end] = if shared_prefix {
                    i
                } else {
                    i.saturating_sub(1)
                };
                bytes
            })
            .collect();
        let (_dir, model, resolver) = fixture(&contents);
        let mut times = [Vec::new(), Vec::new(), Vec::new()];
        let mut bytes_read = [0; 3];
        for _ in 0..3 {
            let started = Instant::now();
            let reference = baseline(&model, &resolver);
            times[0].push(started.elapsed().as_secs_f64() * 1000.0);
            bytes_read[0] = reference.stats.hashed_bytes;
            for (slot, mode) in [
                (1, DuplicateReadMode::Conservative),
                (2, DuplicateReadMode::Parallel),
            ] {
                let started = Instant::now();
                let (actual, _) = run(model.clone(), resolver.clone(), mode);
                times[slot].push(started.elapsed().as_secs_f64() * 1000.0);
                bytes_read[slot] = actual.stats.hashed_bytes;
                assert_eq!(groups(&actual), groups(&reference));
                assert_eq!(actual.total_recoverable, reference.total_recoverable);
            }
        }
        for values in &mut times {
            values.sort_by(f64::total_cmp);
        }
        eprintln!("synthetic shared_prefix={shared_prefix}; files=12; bytes_per_file={SIZE}; debug={}; median_ms baseline={:.3} conservative={:.3} parallel={:.3}; bytes_read={bytes_read:?}", cfg!(debug_assertions), times[0][1], times[1][1], times[2][1]);
    }
}
