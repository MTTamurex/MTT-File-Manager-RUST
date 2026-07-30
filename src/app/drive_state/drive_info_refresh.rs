use std::time::{Duration, Instant};

use crate::domain::file_entry::DriveInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveInfoRefreshScope {
    Local,
    Remote,
}

#[derive(Clone, Debug)]
pub struct DriveInfoRefreshEntry {
    pub path: String,
    pub info: DriveInfo,
    pub capacity_query_succeeded: bool,
}

#[derive(Clone, Debug)]
pub struct DriveInfoRefreshResult {
    pub scope: DriveInfoRefreshScope,
    pub generation: u64,
    pub entries: Vec<DriveInfoRefreshEntry>,
    pub complete: bool,
}

#[derive(Debug)]
struct DriveInfoRefreshSlot {
    in_flight: Option<u64>,
    rerun_requested: bool,
    last_completed: Instant,
}

impl DriveInfoRefreshSlot {
    fn new(now: Instant) -> Self {
        Self {
            in_flight: None,
            rerun_requested: false,
            last_completed: now,
        }
    }
}

#[derive(Debug)]
pub struct DriveInfoRefreshTracker {
    generation: u64,
    local: DriveInfoRefreshSlot,
    remote: DriveInfoRefreshSlot,
}

impl DriveInfoRefreshTracker {
    pub fn new(now: Instant) -> Self {
        Self {
            generation: 0,
            local: DriveInfoRefreshSlot::new(now),
            remote: DriveInfoRefreshSlot::new(now),
        }
    }

    pub fn begin(&mut self, scope: DriveInfoRefreshScope) -> Option<u64> {
        let generation = self.generation;
        let slot = self.slot_mut(scope);
        if slot.in_flight.is_some() {
            slot.rerun_requested = true;
            return None;
        }

        slot.in_flight = Some(generation);
        slot.rerun_requested = false;
        Some(generation)
    }

    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.local.rerun_requested = true;
        self.remote.rerun_requested = true;
    }

    pub fn accepts(&self, scope: DriveInfoRefreshScope, generation: u64) -> bool {
        generation == self.generation && self.slot(scope).in_flight == Some(generation)
    }

    pub fn finish(&mut self, scope: DriveInfoRefreshScope, generation: u64, now: Instant) -> bool {
        let current_generation = self.generation;
        let slot = self.slot_mut(scope);
        if slot.in_flight != Some(generation) {
            return false;
        }

        slot.in_flight = None;
        slot.last_completed = now;
        let rerun = slot.rerun_requested || generation != current_generation;
        slot.rerun_requested = false;
        rerun
    }

    pub fn is_pending(&self, scope: DriveInfoRefreshScope) -> bool {
        self.slot(scope).in_flight.is_some()
    }

    pub fn elapsed(&self, scope: DriveInfoRefreshScope) -> Duration {
        self.slot(scope).last_completed.elapsed()
    }

    fn slot(&self, scope: DriveInfoRefreshScope) -> &DriveInfoRefreshSlot {
        match scope {
            DriveInfoRefreshScope::Local => &self.local,
            DriveInfoRefreshScope::Remote => &self.remote,
        }
    }

    fn slot_mut(&mut self, scope: DriveInfoRefreshScope) -> &mut DriveInfoRefreshSlot {
        match scope {
            DriveInfoRefreshScope::Local => &mut self.local,
            DriveInfoRefreshScope::Remote => &mut self.remote,
        }
    }
}

pub fn merge_drive_info_query(
    existing: Option<&DriveInfo>,
    mut queried: DriveInfo,
    capacity_query_succeeded: bool,
) -> DriveInfo {
    let Some(existing) = existing else {
        return queried;
    };

    if !capacity_query_succeeded {
        queried.total_space = existing.total_space;
        queried.free_space = existing.free_space;
    }
    if queried.file_system.is_empty() {
        queried.file_system.clone_from(&existing.file_system);
    }
    if queried.model.is_none() {
        queried.model.clone_from(&existing.model);
    }
    if queried.serial_number.is_none() {
        queried.serial_number.clone_from(&existing.serial_number);
    }
    if queried.firmware_revision.is_none() {
        queried
            .firmware_revision
            .clone_from(&existing.firmware_revision);
    }
    if queried.bus_type.is_none() {
        queried.bus_type.clone_from(&existing.bus_type);
    }
    queried
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive_info(total_space: u64, free_space: u64) -> DriveInfo {
        DriveInfo {
            file_system: "NTFS".to_string(),
            total_space,
            free_space,
            drive_type: crate::infrastructure::windows::DriveType::Fixed,
            model: Some("model".to_string()),
            serial_number: None,
            firmware_revision: None,
            bus_type: None,
        }
    }

    #[test]
    fn rejects_stale_results_without_overlapping_workers() {
        let mut tracker = DriveInfoRefreshTracker::new(Instant::now());
        let generation = tracker.begin(DriveInfoRefreshScope::Remote).unwrap();

        tracker.invalidate();
        assert_eq!(tracker.begin(DriveInfoRefreshScope::Remote), None);
        assert!(!tracker.accepts(DriveInfoRefreshScope::Remote, generation));
        assert!(tracker.finish(DriveInfoRefreshScope::Remote, generation, Instant::now()));

        let next_generation = tracker.begin(DriveInfoRefreshScope::Remote).unwrap();
        assert_ne!(next_generation, generation);
        assert!(tracker.accepts(DriveInfoRefreshScope::Remote, next_generation));
    }

    #[test]
    fn local_and_remote_refreshes_are_independent() {
        let mut tracker = DriveInfoRefreshTracker::new(Instant::now());

        assert!(tracker.begin(DriveInfoRefreshScope::Remote).is_some());
        assert!(tracker.begin(DriveInfoRefreshScope::Local).is_some());
        assert!(tracker.is_pending(DriveInfoRefreshScope::Remote));
        assert!(tracker.is_pending(DriveInfoRefreshScope::Local));
    }

    #[test]
    fn failed_capacity_query_preserves_last_known_capacity() {
        let existing = drive_info(1_000, 400);
        let mut failed = drive_info(0, 0);
        failed.file_system.clear();

        let merged = merge_drive_info_query(Some(&existing), failed, false);

        assert_eq!(merged.total_space, 1_000);
        assert_eq!(merged.free_space, 400);
        assert_eq!(merged.file_system, "NTFS");
        assert_eq!(merged.model.as_deref(), Some("model"));
    }

    #[test]
    fn successful_capacity_query_preserves_unavailable_metadata() {
        let existing = drive_info(1_000, 400);
        let mut queried = drive_info(2_000, 500);
        queried.file_system.clear();
        queried.model = None;

        let merged = merge_drive_info_query(Some(&existing), queried, true);

        assert_eq!(merged.total_space, 2_000);
        assert_eq!(merged.free_space, 500);
        assert_eq!(merged.file_system, "NTFS");
        assert_eq!(merged.model.as_deref(), Some("model"));
    }
}
