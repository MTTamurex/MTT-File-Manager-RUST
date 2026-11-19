//! Opt-in background sampling. Never read the filesystem or log paths/device IDs.
use super::{CopyOperation, OutcomeKind};
use crate::infrastructure::diagnostic_logger::{
    self, diag_info, diag_warn, field_bool, field_duration_ms, field_label, field_u64,
    DiagnosticField,
};
use mtt_copy_engine::{MoveStrategy, PauseState, Phase, StageTimingSnapshot, TimingSnapshot};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
struct Sample {
    active_time: Duration,
    transferred: u64,
    active: usize,
    phase: Phase,
    running: bool,
}

#[derive(Default)]
struct Probe {
    previous: Option<Sample>,
    stagnant: Duration,
}

impl Probe {
    fn observe(&mut self, current: Sample) -> Duration {
        let measuring =
            current.running && current.active > 0 && current.phase == Phase::Transferring;
        if let Some(previous) = self.previous.filter(|previous| {
            measuring
                && previous.running
                && previous.active > 0
                && previous.phase == current.phase
                && previous.transferred == current.transferred
        }) {
            self.stagnant += current.active_time.saturating_sub(previous.active_time);
        } else {
            self.stagnant = Duration::ZERO;
        }
        self.previous = Some(current);
        self.stagnant
    }
}

pub(crate) struct Monitor {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
    operation: Arc<CopyOperation>,
}

impl Monitor {
    pub(crate) fn start(operation: Arc<CopyOperation>) -> Option<Self> {
        if !diagnostic_logger::is_enabled() {
            return None;
        }
        let timing_operation = operation.clone();
        timing_operation
            .progress
            .telemetry
            .enable_detailed_timings();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = stop.clone();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let thread = std::thread::Builder::new()
            .name("copy-diagnostics".into())
            .spawn(move || {
                let progress = &operation.progress;
                let telemetry = &progress.telemetry;
                let mut probe = Probe::default();
                let mut policy_recorded = false;
                let mut last_sample = Instant::now();
                diag_info(
                    "copy_engine",
                    "started",
                    &[
                        field_u64("copy_id", id),
                        field_label("operation", if operation.moving { "move" } else { "copy" }),
                        field_label("cache", "buffered"),
                        field_label("priority", "unchanged"),
                    ],
                );
                loop {
                    let (lock, wake) = &*signal;
                    let stopped = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let (stopped, _) = wake
                        .wait_timeout_while(stopped, Duration::from_secs(2), |stop| !*stop)
                        .unwrap_or_else(|e| e.into_inner());
                    if *stopped {
                        break;
                    }
                    drop(stopped);
                    if !diagnostic_logger::is_enabled() {
                        last_sample = Instant::now();
                        continue;
                    }
                    if let Some(policy) = telemetry.policy().filter(|_| !policy_recorded) {
                        diag_info(
                            "copy_engine",
                            "policy",
                            &[
                                field_u64("copy_id", id),
                                field_u64("workers", policy.workers as u64),
                                field_u64("large_workers", policy.large_workers as u64),
                                field_bool("shared_device", policy.shared_device),
                                field_bool("system_device", policy.system_device),
                                field_bool("conservative", policy.conservative),
                            ],
                        );
                        policy_recorded = true;
                    }
                    let phase = telemetry.phase();
                    let sample = Sample {
                        active_time: progress.active_elapsed(),
                        transferred: telemetry.transferred.load(Ordering::Relaxed),
                        active: telemetry.active.load(Ordering::Relaxed),
                        phase,
                        running: progress.pause_state() == PauseState::Running
                            && !progress.is_cancelled(),
                    };
                    let stagnant = probe.observe(sample);
                    let sample_interval = last_sample.elapsed();
                    last_sample = Instant::now();
                    let mut fields = vec![
                        field_duration_ms("sample_interval", sample_interval),
                        field_u64("copy_id", id),
                        field_label(
                            "phase",
                            match phase {
                                Phase::Preparing => "preparing",
                                Phase::Profiling => "profiling",
                                Phase::Transferring => "transferring",
                                Phase::Finished => "finished",
                            },
                        ),
                        field_u64("api_bytes", sample.transferred),
                        field_u64(
                            "committed_and_inflight_bytes",
                            progress.bytes_copied.load(Ordering::Relaxed),
                        ),
                        field_u64("active_transfers", sample.active as u64),
                        field_u64(
                            "waiting_for_budget",
                            telemetry.waiting.load(Ordering::Relaxed) as u64,
                        ),
                        field_u64(
                            "completed_files",
                            progress.files_copied.load(Ordering::Relaxed) as u64,
                        ),
                        field_duration_ms("active_time", sample.active_time),
                        field_duration_ms("no_advance", stagnant),
                        field_bool("running", sample.running),
                    ];
                    append_operation_fields(&mut fields, &operation);
                    append_timing_fields(&mut fields, telemetry.timing_snapshot());
                    if stagnant >= Duration::from_secs(4) {
                        diag_warn("copy_engine", "progress_sample", &fields);
                    } else {
                        diag_info("copy_engine", "progress_sample", &fields);
                    }
                }
                let mut fields = vec![
                    field_u64("copy_id", id),
                    field_u64("api_bytes", telemetry.transferred.load(Ordering::Relaxed)),
                    field_u64(
                        "peak_transfers",
                        telemetry.peak_active.load(Ordering::Relaxed) as u64,
                    ),
                    field_bool("cancelled", progress.is_cancelled()),
                    field_u64(
                        "completed_files",
                        progress.files_copied.load(Ordering::Relaxed) as u64,
                    ),
                    field_label(
                        "outcome",
                        match operation.outcome().map(|outcome| outcome.kind) {
                            Some(OutcomeKind::Completed) => "completed",
                            Some(OutcomeKind::Cancelled) => "cancelled",
                            Some(OutcomeKind::Failed) => "failed",
                            None => "interrupted",
                        },
                    ),
                ];
                if let Some(policy) = telemetry.policy() {
                    fields.extend([
                        field_u64("workers", policy.workers as u64),
                        field_u64("large_workers", policy.large_workers as u64),
                        field_bool("shared_device", policy.shared_device),
                        field_bool("system_device", policy.system_device),
                        field_bool("conservative", policy.conservative),
                    ]);
                }
                append_operation_fields(&mut fields, &operation);
                append_timing_fields(&mut fields, telemetry.timing_snapshot());
                diag_info("copy_engine", "finished", &fields);
            });
        let thread = match thread {
            Ok(thread) => thread,
            Err(_) => {
                timing_operation
                    .progress
                    .telemetry
                    .disable_detailed_timings();
                return None;
            }
        };
        Some(Self {
            stop,
            thread: Some(thread),
            operation: timing_operation,
        })
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        let (lock, wake) = &*self.stop;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        wake.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.operation.progress.telemetry.disable_detailed_timings();
    }
}

fn append_stage_timing(
    fields: &mut Vec<DiagnosticField>,
    count_name: &'static str,
    total_name: &'static str,
    max_name: &'static str,
    timing: StageTimingSnapshot,
) {
    fields.push(field_u64(count_name, timing.count));
    fields.push(field_duration_ms(total_name, timing.total));
    fields.push(field_duration_ms(max_name, timing.max));
}

fn append_timing_fields(fields: &mut Vec<DiagnosticField>, timings: TimingSnapshot) {
    append_stage_timing(
        fields,
        "preparation_count",
        "preparation_total_ms",
        "preparation_max_ms",
        timings.preparation,
    );
    append_stage_timing(
        fields,
        "file_setup_count",
        "file_setup_total_ms",
        "file_setup_max_ms",
        timings.file_setup,
    );
    append_stage_timing(
        fields,
        "staging_create_count",
        "staging_create_total_ms",
        "staging_create_max_ms",
        timings.staging_create,
    );
    append_stage_timing(
        fields,
        "copy_api_count",
        "copy_api_total_ms",
        "copy_api_max_ms",
        timings.copy_api,
    );
    append_stage_timing(
        fields,
        "first_progress_count",
        "first_progress_total_ms",
        "first_progress_max_ms",
        timings.first_progress,
    );
    append_stage_timing(
        fields,
        "copy_verify_count",
        "copy_verify_total_ms",
        "copy_verify_max_ms",
        timings.copy_verify,
    );
    append_stage_timing(
        fields,
        "copy_publish_count",
        "copy_publish_total_ms",
        "copy_publish_max_ms",
        timings.copy_publish,
    );
    append_stage_timing(
        fields,
        "move_setup_count",
        "move_setup_total_ms",
        "move_setup_max_ms",
        timings.move_setup,
    );
    append_stage_timing(
        fields,
        "move_rename_count",
        "move_rename_total_ms",
        "move_rename_max_ms",
        timings.move_rename,
    );
    append_stage_timing(
        fields,
        "move_finalize_count",
        "move_finalize_total_ms",
        "move_finalize_max_ms",
        timings.move_finalize,
    );
    append_stage_timing(
        fields,
        "source_remove_count",
        "source_remove_total_ms",
        "source_remove_max_ms",
        timings.source_remove,
    );
}

fn append_operation_fields(fields: &mut Vec<DiagnosticField>, operation: &CopyOperation) {
    let telemetry = &operation.progress.telemetry;
    let inventory = telemetry.inventory_snapshot();
    fields.extend([
        field_label("operation", if operation.moving { "move" } else { "copy" }),
        field_u64("examined_directories", inventory.directories),
        field_u64("examined_files", inventory.files),
        field_u64("discovered_file_bytes", inventory.file_bytes),
    ]);
    if operation.moving {
        let strategy: MoveStrategy = telemetry.move_strategy();
        fields.extend([
            field_label("move_strategy", strategy.as_label()),
            field_u64(
                "cross_volume_directory_fallbacks",
                telemetry.cross_volume_directory_fallbacks(),
            ),
            field_u64(
                "cross_volume_file_fallbacks",
                telemetry.cross_volume_file_fallbacks(),
            ),
        ]);
    }
    let first_copy_progress = telemetry.first_copy_progress_time();
    fields.extend([
        field_bool(
            "first_copy_progress_observed",
            first_copy_progress.is_some(),
        ),
        field_duration_ms(
            "time_to_first_copy_progress_ms",
            first_copy_progress.unwrap_or_default(),
        ),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seconds: u64, bytes: u64) -> Sample {
        Sample {
            active_time: Duration::from_secs(seconds),
            transferred: bytes,
            active: 1,
            phase: Phase::Transferring,
            running: true,
        }
    }

    #[test]
    fn no_progress_is_distinct_from_pause_preflight_and_budget_waits() {
        let mut probe = Probe::default();
        assert_eq!(probe.observe(sample(0, 10)), Duration::ZERO);
        assert_eq!(probe.observe(sample(2, 10)), Duration::from_secs(2));
        assert_eq!(probe.observe(sample(4, 10)), Duration::from_secs(4));
        assert_eq!(probe.observe(sample(6, 20)), Duration::ZERO);
        assert_eq!(
            probe.observe(Sample {
                running: false,
                ..sample(6, 20)
            }),
            Duration::ZERO
        );
        assert_eq!(probe.observe(sample(7, 20)), Duration::ZERO);
        assert_eq!(
            probe.observe(Sample {
                active: 0,
                ..sample(9, 20)
            }),
            Duration::ZERO
        );
        assert_eq!(
            probe.observe(Sample {
                phase: Phase::Profiling,
                ..sample(11, 20)
            }),
            Duration::ZERO
        );
    }
}
