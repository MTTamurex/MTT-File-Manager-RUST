use std::time::{Duration, Instant};

use mtt_search_protocol::DriveHealthSnapshot;

use super::{normalize_drive_root_key, DriveState};

const DRIVE_HEALTH_SUCCESS_TTL: Duration = Duration::from_secs(300);
const DRIVE_HEALTH_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

mod scheduler;
pub use scheduler::{DriveHealthRequestKind, DriveHealthScheduler, ScheduledDriveHealthRequest};

#[derive(Debug)]
pub struct DriveHealthResult {
    pub root: String,
    pub request_id: u64,
    pub completed_at: Instant,
    pub result: Result<DriveHealthSnapshot, String>,
}

impl DriveState {
    pub fn can_begin_drive_health_request(&self, path: &str, now: Instant) -> bool {
        let Some(root) = normalize_drive_root_key(path) else {
            return false;
        };
        !self.drive_health_pending.contains_key(&root)
            && !self
                .drive_health_failed_at
                .get(&root)
                .is_some_and(|failed| {
                    now.saturating_duration_since(*failed) < DRIVE_HEALTH_FAILURE_COOLDOWN
                })
            && !self
                .drive_health_updated_at
                .get(&root)
                .is_some_and(|updated| {
                    now.saturating_duration_since(*updated) < DRIVE_HEALTH_SUCCESS_TTL
                })
    }

    pub fn begin_drive_health_request(&mut self, path: &str, now: Instant) -> Option<u64> {
        let root = normalize_drive_root_key(path)?;
        if !self.can_begin_drive_health_request(&root, now) {
            return None;
        }

        let request_id = self.drive_health_next_request_id;
        self.drive_health_next_request_id = self.drive_health_next_request_id.wrapping_add(1);
        self.drive_health_pending.insert(root, request_id);
        Some(request_id)
    }

    pub fn finish_drive_health_request(&mut self, path: &str, request_id: u64) -> bool {
        let Some(root) = normalize_drive_root_key(path) else {
            return false;
        };
        if self.drive_health_pending.get(&root) != Some(&request_id) {
            return false;
        }
        self.drive_health_pending.remove(&root);
        true
    }

    pub fn cache_drive_health(&mut self, path: &str, snapshot: DriveHealthSnapshot, now: Instant) {
        let Some(root) = normalize_drive_root_key(path) else {
            return;
        };
        self.drive_health_cache.insert(root.clone(), snapshot);
        self.drive_health_updated_at.insert(root.clone(), now);
        self.drive_health_failed_at.remove(&root);
    }

    pub fn cached_drive_health(&self, path: &str, now: Instant) -> Option<DriveHealthSnapshot> {
        let root = normalize_drive_root_key(path)?;
        self.drive_health_updated_at.get(&root).filter(|updated| {
            now.saturating_duration_since(**updated) < DRIVE_HEALTH_SUCCESS_TTL
        })?;
        self.drive_health_cache.get(&root).cloned()
    }

    pub fn remove_drive_health_snapshot(
        &mut self,
        path: &str,
    ) -> Option<crate::domain::file_entry::DriveInfo> {
        let root = normalize_drive_root_key(path)?;
        self.drive_health_cache.remove(&root);
        self.drive_health_updated_at.remove(&root);
        let info = self.drive_info_cache.get_mut(&root)?;
        info.health = None;
        self.drive_info_cache_epoch = self.drive_info_cache_epoch.wrapping_add(1);
        Some(info.clone())
    }

    pub fn record_drive_health_failure(&mut self, path: &str, now: Instant) {
        if let Some(root) = normalize_drive_root_key(path) {
            self.drive_health_failed_at.insert(root, now);
        }
    }

    pub fn invalidate_drive_health(&mut self, path: &str) {
        let Some(root) = normalize_drive_root_key(path) else {
            return;
        };
        self.drive_health_cache.remove(&root);
        self.drive_health_pending.remove(&root);
        self.drive_health_updated_at.remove(&root);
        self.drive_health_failed_at.remove(&root);
    }

    pub fn clear_drive_health(&mut self) {
        self.drive_health_cache.clear();
        self.drive_health_pending.clear();
        self.drive_health_updated_at.clear();
        self.drive_health_failed_at.clear();
    }
}
