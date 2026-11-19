//! Numeric-only observations. Transfer bytes are API progress, not disk flushes.
use crate::PolicyInfo;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

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
    detailed_timings_enabled: AtomicBool,
    timings: Timings,
    inventory: Inventory,
    move_strategy: AtomicUsize,
    cross_volume_directory_fallbacks: AtomicU64,
    cross_volume_file_fallbacks: AtomicU64,
    first_copy_progress_micros_plus_one: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InventorySnapshot {
    pub directories: u64,
    pub files: u64,
    /// Logical bytes discovered in regular files, including skipped conflicts.
    pub file_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MoveStrategy {
    #[default]
    NotApplicable,
    Undetermined,
    PerEntry,
    FastDirectoryRename,
}

impl MoveStrategy {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::Undetermined => "undetermined",
            Self::PerEntry => "per_entry",
            Self::FastDirectoryRename => "fast_directory_rename",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StageTimingSnapshot {
    pub count: u64,
    pub total: Duration,
    pub max: Duration,
}

/// Privacy-safe stage timings, aggregated across files in one operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimingSnapshot {
    pub preparation: StageTimingSnapshot,
    pub file_setup: StageTimingSnapshot,
    pub staging_create: StageTimingSnapshot,
    pub copy_api: StageTimingSnapshot,
    pub first_progress: StageTimingSnapshot,
    pub copy_verify: StageTimingSnapshot,
    pub copy_publish: StageTimingSnapshot,
    pub move_setup: StageTimingSnapshot,
    pub move_rename: StageTimingSnapshot,
    pub move_finalize: StageTimingSnapshot,
    pub source_remove: StageTimingSnapshot,
}

#[derive(Default)]
struct Timings {
    preparation: StageTiming,
    file_setup: StageTiming,
    staging_create: StageTiming,
    copy_api: StageTiming,
    first_progress: StageTiming,
    copy_verify: StageTiming,
    copy_publish: StageTiming,
    move_setup: StageTiming,
    move_rename: StageTiming,
    move_finalize: StageTiming,
    source_remove: StageTiming,
}

#[derive(Default)]
struct Inventory {
    directories: AtomicU64,
    files: AtomicU64,
    file_bytes: AtomicU64,
}

#[derive(Default)]
struct StageTiming {
    count: AtomicU64,
    total_micros: AtomicU64,
    max_micros: AtomicU64,
}

#[derive(Clone, Copy)]
pub(crate) enum TimingStage {
    Preparation,
    FileSetup,
    StagingCreate,
    CopyApi,
    FirstProgress,
    CopyVerify,
    CopyPublish,
    MoveSetup,
    MoveRename,
    MoveFinalize,
    SourceRemove,
}

impl StageTiming {
    fn record(&self, duration: Duration) {
        let micros = duration.as_micros().try_into().unwrap_or(u64::MAX);
        let _ = self
            .total_micros
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |total| {
                Some(total.saturating_add(micros))
            });
        self.max_micros.fetch_max(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> StageTimingSnapshot {
        StageTimingSnapshot {
            count: self.count.load(Ordering::Relaxed),
            total: Duration::from_micros(self.total_micros.load(Ordering::Relaxed)),
            max: Duration::from_micros(self.max_micros.load(Ordering::Relaxed)),
        }
    }
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
    pub fn timing_snapshot(&self) -> TimingSnapshot {
        TimingSnapshot {
            preparation: self.timings.preparation.snapshot(),
            file_setup: self.timings.file_setup.snapshot(),
            staging_create: self.timings.staging_create.snapshot(),
            copy_api: self.timings.copy_api.snapshot(),
            first_progress: self.timings.first_progress.snapshot(),
            copy_verify: self.timings.copy_verify.snapshot(),
            copy_publish: self.timings.copy_publish.snapshot(),
            move_setup: self.timings.move_setup.snapshot(),
            move_rename: self.timings.move_rename.snapshot(),
            move_finalize: self.timings.move_finalize.snapshot(),
            source_remove: self.timings.source_remove.snapshot(),
        }
    }
    pub fn inventory_snapshot(&self) -> InventorySnapshot {
        InventorySnapshot {
            directories: self.inventory.directories.load(Ordering::Relaxed),
            files: self.inventory.files.load(Ordering::Relaxed),
            file_bytes: self.inventory.file_bytes.load(Ordering::Relaxed),
        }
    }
    pub fn move_strategy(&self) -> MoveStrategy {
        match self.move_strategy.load(Ordering::Relaxed) {
            1 => MoveStrategy::Undetermined,
            2 => MoveStrategy::PerEntry,
            3 => MoveStrategy::FastDirectoryRename,
            _ => MoveStrategy::NotApplicable,
        }
    }
    pub fn cross_volume_directory_fallbacks(&self) -> u64 {
        self.cross_volume_directory_fallbacks
            .load(Ordering::Relaxed)
    }
    pub fn cross_volume_file_fallbacks(&self) -> u64 {
        self.cross_volume_file_fallbacks.load(Ordering::Relaxed)
    }
    /// Time from operation creation to the first positive copy callback, if any.
    pub fn first_copy_progress_time(&self) -> Option<Duration> {
        self.first_copy_progress_micros_plus_one
            .load(Ordering::Relaxed)
            .checked_sub(1)
            .map(Duration::from_micros)
    }
    pub(crate) fn record_examined_directory(&self) {
        if self.detailed_timings_enabled() {
            self.inventory.directories.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn record_examined_file(&self, bytes: u64) {
        if self.detailed_timings_enabled() {
            self.inventory.files.fetch_add(1, Ordering::Relaxed);
            let _ = self.inventory.file_bytes.try_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |total| Some(total.saturating_add(bytes)),
            );
        }
    }
    pub(crate) fn record_cross_volume_directory_fallback(&self) {
        if self.detailed_timings_enabled() {
            self.cross_volume_directory_fallbacks
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn record_cross_volume_file_fallback(&self) {
        if self.detailed_timings_enabled() {
            self.cross_volume_file_fallbacks
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn record_first_copy_progress(&self, elapsed: Duration) {
        if self.detailed_timings_enabled() {
            let micros = elapsed.as_micros().try_into().unwrap_or(u64::MAX - 1);
            let _ = self.first_copy_progress_micros_plus_one.compare_exchange(
                0,
                micros.saturating_add(1),
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
    /// Enables per-file stage timing collection for the current operation.
    pub fn enable_detailed_timings(&self) {
        self.detailed_timings_enabled.store(true, Ordering::Release);
    }
    /// Stops per-file stage timing collection.
    pub fn disable_detailed_timings(&self) {
        self.detailed_timings_enabled
            .store(false, Ordering::Release);
    }
    pub(crate) fn detailed_timings_enabled(&self) -> bool {
        self.detailed_timings_enabled.load(Ordering::Acquire)
    }
    pub(crate) fn measure<'a>(
        &'a self,
        progress: &'a crate::Progress,
        stage: TimingStage,
    ) -> Option<StageTimer<'a>> {
        self.detailed_timings_enabled().then(|| StageTimer {
            telemetry: self,
            progress,
            stage,
            started: progress.active_elapsed(),
        })
    }
    pub(crate) fn record_timing(&self, stage: TimingStage, duration: Duration) {
        self.timings.for_stage(stage).record(duration);
    }
    pub(crate) fn set_policy(&self, policy: PolicyInfo) {
        *self.policy.lock().unwrap_or_else(|e| e.into_inner()) = Some(policy);
    }
    pub(crate) fn set_move_strategy(&self, strategy: MoveStrategy) {
        self.move_strategy.store(
            match strategy {
                MoveStrategy::NotApplicable => 0,
                MoveStrategy::Undetermined => 1,
                MoveStrategy::PerEntry => 2,
                MoveStrategy::FastDirectoryRename => 3,
            },
            Ordering::Relaxed,
        );
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

impl Timings {
    fn for_stage(&self, stage: TimingStage) -> &StageTiming {
        match stage {
            TimingStage::Preparation => &self.preparation,
            TimingStage::FileSetup => &self.file_setup,
            TimingStage::StagingCreate => &self.staging_create,
            TimingStage::CopyApi => &self.copy_api,
            TimingStage::FirstProgress => &self.first_progress,
            TimingStage::CopyVerify => &self.copy_verify,
            TimingStage::CopyPublish => &self.copy_publish,
            TimingStage::MoveSetup => &self.move_setup,
            TimingStage::MoveRename => &self.move_rename,
            TimingStage::MoveFinalize => &self.move_finalize,
            TimingStage::SourceRemove => &self.source_remove,
        }
    }
}

pub(crate) struct StageTimer<'a> {
    telemetry: &'a Telemetry,
    progress: &'a crate::Progress,
    stage: TimingStage,
    started: Duration,
}

impl Drop for StageTimer<'_> {
    fn drop(&mut self) {
        let elapsed = self.progress.active_elapsed().saturating_sub(self.started);
        self.telemetry.record_timing(self.stage, elapsed);
    }
}

pub(crate) struct Run<'a>(&'a Telemetry);
impl Drop for Run<'_> {
    fn drop(&mut self) {
        self.0.set_phase(Phase::Finished);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detailed_timing_collection_is_opt_in() {
        let progress = crate::Progress::default();
        let telemetry = Telemetry::default();

        assert!(telemetry
            .measure(&progress, TimingStage::FileSetup)
            .is_none());
        assert_eq!(telemetry.timing_snapshot().file_setup.count, 0);

        telemetry.enable_detailed_timings();
        drop(telemetry.measure(&progress, TimingStage::FileSetup));

        let setup = telemetry.timing_snapshot().file_setup;
        assert_eq!(setup.count, 1);
        assert!(setup.total >= setup.max);
    }

    #[test]
    fn inventory_and_fallback_metrics_are_opt_in_and_aggregated() {
        let telemetry = Telemetry::default();
        telemetry.record_examined_directory();
        telemetry.record_examined_file(5);
        telemetry.record_cross_volume_file_fallback();
        telemetry.record_first_copy_progress(Duration::from_millis(10));
        assert_eq!(telemetry.inventory_snapshot(), InventorySnapshot::default());
        assert_eq!(telemetry.cross_volume_file_fallbacks(), 0);
        assert_eq!(telemetry.first_copy_progress_time(), None);

        telemetry.enable_detailed_timings();
        telemetry.record_examined_directory();
        telemetry.record_examined_file(5);
        telemetry.record_examined_file(7);
        telemetry.record_cross_volume_file_fallback();
        telemetry.record_cross_volume_file_fallback();
        telemetry.record_first_copy_progress(Duration::from_millis(10));
        telemetry.record_first_copy_progress(Duration::from_millis(20));

        assert_eq!(
            telemetry.inventory_snapshot(),
            InventorySnapshot {
                directories: 1,
                files: 2,
                file_bytes: 12,
            }
        );
        assert_eq!(telemetry.cross_volume_file_fallbacks(), 2);
        assert_eq!(
            telemetry.first_copy_progress_time(),
            Some(Duration::from_millis(10))
        );
    }

    #[test]
    fn timing_metrics_accumulate_totals_and_maxima() {
        let timing = StageTiming::default();
        timing.record(Duration::from_millis(12));
        timing.record(Duration::from_millis(5));

        assert_eq!(
            timing.snapshot(),
            StageTimingSnapshot {
                count: 2,
                total: Duration::from_millis(17),
                max: Duration::from_millis(12),
            }
        );
    }
}
