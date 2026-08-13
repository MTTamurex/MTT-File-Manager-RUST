use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

const PRELOAD_DELAY: Duration = Duration::from_secs(5);
const INTER_QUERY_DELAY: Duration = Duration::from_secs(1);
const RETRY_DELAY: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveHealthRequestKind {
    Interactive,
    Preload,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ScheduledDriveHealthRequest {
    pub root: String,
    pub kind: DriveHealthRequestKind,
}

#[derive(Debug)]
pub struct DriveHealthScheduler {
    preload_not_before: Option<Instant>,
    preload_reconcile_needed: bool,
    next_preload_not_before: Option<Instant>,
    retry_not_before: Option<Instant>,
    preload_queue: VecDeque<String>,
    preload_queued: HashSet<String>,
    preload_seen: HashSet<String>,
    interactive_target: Option<String>,
    active: Option<(String, u64, DriveHealthRequestKind)>,
}

impl DriveHealthScheduler {
    pub fn new() -> Self {
        Self {
            preload_not_before: None,
            preload_reconcile_needed: false,
            next_preload_not_before: None,
            retry_not_before: None,
            preload_queue: VecDeque::new(),
            preload_queued: HashSet::new(),
            preload_seen: HashSet::new(),
            interactive_target: None,
            active: None,
        }
    }

    pub fn arm_preload(&mut self, now: Instant) {
        self.preload_not_before = Some(now + PRELOAD_DELAY);
        self.preload_reconcile_needed = true;
    }

    pub fn reset_preload(&mut self, now: Instant) {
        self.preload_queue.clear();
        self.preload_queued.clear();
        self.preload_seen.clear();
        self.next_preload_not_before = None;
        self.retry_not_before = None;
        self.arm_preload(now);
    }

    pub fn preload_is_due(&self, now: Instant) -> bool {
        self.preload_not_before
            .is_some_and(|deadline| now >= deadline)
    }

    pub fn next_wakeup_in(&self, now: Instant) -> Option<Duration> {
        if let Some(deadline) = self.retry_not_before.filter(|deadline| *deadline > now) {
            return Some(deadline.duration_since(now));
        }
        [self.preload_not_before, self.next_preload_not_before]
            .into_iter()
            .flatten()
            .filter(|deadline| *deadline > now)
            .map(|deadline| deadline.duration_since(now))
            .min()
    }

    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn preload_reconcile_needed(&self) -> bool {
        self.preload_reconcile_needed
    }

    pub fn reconcile_preload(&mut self, roots: impl IntoIterator<Item = String>) {
        self.preload_reconcile_needed = false;
        let roots: Vec<String> = roots.into_iter().collect();
        let current_roots: HashSet<&str> = roots.iter().map(String::as_str).collect();
        self.preload_queue
            .retain(|root| current_roots.contains(root.as_str()));
        self.preload_queued
            .retain(|root| current_roots.contains(root.as_str()));
        self.preload_seen
            .retain(|root| current_roots.contains(root.as_str()));

        for root in roots {
            if self.active.as_ref().is_some_and(|active| active.0 == root)
                || self.interactive_target.as_ref() == Some(&root)
            {
                continue;
            }
            if !self.preload_seen.insert(root.clone()) || !self.preload_queued.insert(root.clone())
            {
                continue;
            }
            self.preload_queue.push_back(root);
        }
    }

    pub fn set_interactive_target(&mut self, root: Option<String>) {
        if self.interactive_target != root {
            if let Some(previous) = self.interactive_target.take() {
                if self.preload_seen.contains(&previous)
                    && !self.preload_queued.contains(&previous)
                    && self
                        .active
                        .as_ref()
                        .is_none_or(|active| active.0 != previous)
                {
                    self.preload_queued.insert(previous.clone());
                    self.preload_queue.push_front(previous);
                }
            }
        }
        let root = root.filter(|root| self.active.as_ref().is_none_or(|active| active.0 != *root));
        if let Some(root) = &root {
            if self.preload_queued.remove(root) {
                self.preload_queue.retain(|queued| queued != root);
            }
        }
        self.interactive_target = root;
    }

    pub fn take_next(
        &mut self,
        now: Instant,
        allow_preload: bool,
    ) -> Option<ScheduledDriveHealthRequest> {
        if self.active.is_some() || self.retry_not_before.is_some_and(|deadline| now < deadline) {
            return None;
        }
        if let Some(root) = self.interactive_target.take() {
            return Some(ScheduledDriveHealthRequest {
                root,
                kind: DriveHealthRequestKind::Interactive,
            });
        }
        if !allow_preload
            || self
                .next_preload_not_before
                .is_some_and(|deadline| now < deadline)
        {
            return None;
        }

        let root = self.preload_queue.pop_front()?;
        self.preload_queued.remove(&root);
        Some(ScheduledDriveHealthRequest {
            root,
            kind: DriveHealthRequestKind::Preload,
        })
    }

    pub fn mark_active(&mut self, root: String, request_id: u64, kind: DriveHealthRequestKind) {
        debug_assert!(self.active.is_none());
        self.active = Some((root, request_id, kind));
    }

    pub fn finish_active(
        &mut self,
        root: &str,
        request_id: u64,
        completed_at: Instant,
    ) -> Option<DriveHealthRequestKind> {
        let active = self.active.take()?;
        if active.0 != root || active.1 != request_id {
            self.active = Some(active);
            return None;
        }
        self.next_preload_not_before = Some(completed_at + INTER_QUERY_DELAY);
        self.preload_reconcile_needed = true;
        Some(active.2)
    }

    pub fn defer(&mut self, root: String, kind: DriveHealthRequestKind, now: Instant) {
        self.retry_not_before = Some(now + RETRY_DELAY);
        match kind {
            DriveHealthRequestKind::Interactive => self.interactive_target = Some(root),
            DriveHealthRequestKind::Preload => {
                if self.preload_queued.insert(root.clone()) {
                    self.preload_queue.push_front(root);
                }
            }
        }
    }
}

impl Default for DriveHealthScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preload_waits_for_startup_delay() {
        let start = Instant::now();
        let mut scheduler = DriveHealthScheduler::new();
        scheduler.arm_preload(start);

        assert!(!scheduler.preload_is_due(start + Duration::from_secs(4)));
        assert_eq!(scheduler.next_wakeup_in(start), Some(PRELOAD_DELAY));
        assert!(scheduler.preload_is_due(start + Duration::from_secs(5)));
    }

    #[test]
    fn requests_are_serial_and_interactive_target_is_promoted() {
        let start = Instant::now();
        let mut scheduler = DriveHealthScheduler::new();
        scheduler.reconcile_preload(["C:\\".to_string(), "D:\\".to_string(), "E:\\".to_string()]);

        let first = scheduler.take_next(start, true).unwrap();
        assert_eq!(first.root, "C:\\");
        assert_eq!(first.kind, DriveHealthRequestKind::Preload);
        scheduler.mark_active(first.root, 1, first.kind);

        scheduler.set_interactive_target(Some("D:\\".to_string()));
        assert_eq!(scheduler.take_next(start, true), None);
        assert_eq!(
            scheduler.finish_active("C:\\", 1, start),
            Some(DriveHealthRequestKind::Preload)
        );

        let promoted = scheduler.take_next(start, true).unwrap();
        assert_eq!(promoted.root, "D:\\");
        assert_eq!(promoted.kind, DriveHealthRequestKind::Interactive);
        scheduler.mark_active(promoted.root, 2, promoted.kind);
        assert_eq!(
            scheduler.finish_active("D:\\", 2, start),
            Some(DriveHealthRequestKind::Interactive)
        );

        assert_eq!(scheduler.take_next(start, true), None);
        assert_eq!(
            scheduler
                .take_next(start + INTER_QUERY_DELAY, true)
                .unwrap()
                .root,
            "E:\\"
        );
    }

    #[test]
    fn preload_pauses_and_reconciles_removed_drives() {
        let start = Instant::now();
        let mut scheduler = DriveHealthScheduler::new();
        scheduler.reconcile_preload(["C:\\".to_string(), "D:\\".to_string()]);
        scheduler.reconcile_preload(["D:\\".to_string(), "E:\\".to_string()]);

        assert_eq!(scheduler.take_next(start, false), None);
        assert_eq!(scheduler.take_next(start, true).unwrap().root, "D:\\");
        assert_eq!(scheduler.take_next(start, true).unwrap().root, "E:\\");
        assert_eq!(scheduler.take_next(start, true), None);
    }

    #[test]
    fn temporary_service_busy_requeues_and_backs_off() {
        let start = Instant::now();
        let mut scheduler = DriveHealthScheduler::new();
        scheduler.reconcile_preload(["C:\\".to_string()]);
        let request = scheduler.take_next(start, true).unwrap();
        scheduler.mark_active(request.root.clone(), 1, request.kind);
        let kind = scheduler.finish_active("C:\\", 1, start).unwrap();
        scheduler.defer(request.root, kind, start);

        assert_eq!(scheduler.take_next(start, true), None);
        assert_eq!(scheduler.next_wakeup_in(start), Some(RETRY_DELAY));
        assert_eq!(
            scheduler.take_next(start + RETRY_DELAY, true).unwrap().root,
            "C:\\"
        );
    }

    #[test]
    fn completing_an_active_request_reconciles_drives_seen_during_it() {
        let start = Instant::now();
        let mut scheduler = DriveHealthScheduler::new();
        scheduler.reconcile_preload(["C:\\".to_string()]);
        let request = scheduler.take_next(start, true).unwrap();
        scheduler.mark_active(request.root, 1, request.kind);

        scheduler.reset_preload(start);
        scheduler.reconcile_preload(["C:\\".to_string()]);
        assert!(!scheduler.preload_reconcile_needed());
        assert_eq!(
            scheduler.finish_active("C:\\", 1, start),
            Some(DriveHealthRequestKind::Preload)
        );
        assert!(scheduler.preload_reconcile_needed());
    }
}
