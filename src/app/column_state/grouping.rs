//! Navigation-scoped, asynchronous full-list grouping and interaction-safe publication.
use super::{
    grouping_worker::{run, GroupRequest},
    ColumnState,
};
use crate::{
    app::ImageViewerApp,
    application::grouping::{columns::ColumnGroups, GroupProjection},
    domain::{
        file_entry::{FileEntry, GroupMode},
        list_columns::ColumnId,
    },
};
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
mod app;
mod miller;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::Instant,
};

struct GroupJob {
    cancelled: Arc<AtomicBool>,
    latest: Option<Arc<ColumnGroups>>,
    published: HashMap<GroupOwner, Published>,
    in_flight: bool,
    retry_at: Option<Instant>,
}

impl GroupJob {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            latest: None,
            published: HashMap::new(),
            in_flight: true,
            retry_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum GroupOwner {
    Main(usize),
    Ancestor(usize, PathBuf),
}

struct Published {
    groups: Arc<ColumnGroups>,
    projection: Weak<GroupProjection>,
}
impl Drop for GroupJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) struct ColumnGrouping {
    sender: Option<Sender<GroupRequest>>,
    receiver: Receiver<(u64, ColumnGroups)>,
    jobs: HashMap<u64, GroupJob>,
    current: HashMap<GroupOwner, u64>,
}

impl ColumnGrouping {
    pub(super) fn new() -> Self {
        let (sender, requests) = bounded::<GroupRequest>(2);
        let (results, receiver) = bounded(4);
        let started = crate::spawn_named("column-groups", move || {
            while let Ok(request) = requests.recv() {
                if request.cancelled.load(Ordering::Acquire) {
                    continue;
                }
                let grouped = run(&request).unwrap_or_default();
                if !request.cancelled.load(Ordering::Acquire) {
                    if results.send((request.signature, grouped)).is_err() {
                        break;
                    }
                    request.ctx.request_repaint();
                }
            }
        })
        .is_ok();
        Self {
            sender: started.then_some(sender),
            receiver,
            jobs: HashMap::new(),
            current: HashMap::new(),
        }
    }

    fn set_current(&mut self, pane: usize, signature: Option<u64>) -> bool {
        self.set_owner(GroupOwner::Main(pane), signature)
    }

    fn set_owner(&mut self, owner: GroupOwner, signature: Option<u64>) -> bool {
        if self.current.get(&owner).copied() == signature {
            return false;
        }
        let old = self.current.remove(&owner);
        if let Some(signature) = signature {
            self.current.insert(owner.clone(), signature);
        }
        if let Some(old) = old {
            if let Some(job) = self.jobs.get_mut(&old) {
                job.published.remove(&owner);
            }
            if !self.current.values().any(|&signature| signature == old) {
                self.jobs.remove(&old);
            }
        }
        true
    }

    fn retain_ancestors(&mut self, pane: usize, keep: &HashSet<PathBuf>) {
        let obsolete: Vec<_> = self.current.keys().filter(|owner| matches!(owner, GroupOwner::Ancestor(owner_pane, path) if *owner_pane == pane && !keep.contains(path))).cloned().collect();
        for owner in obsolete {
            self.set_owner(owner, None);
        }
    }

    fn poll(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok((signature, result)) => {
                    if let Some(job) = self.jobs.get_mut(&signature) {
                        job.retry_at = result
                            .deferred
                            .then_some(Instant::now() + super::RETRY_DELAY);
                        job.latest = Some(Arc::new(result));
                        job.in_flight = false;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.sender = None;
                    self.current.clear();
                    self.jobs.clear();
                    break;
                }
            }
        }
    }

    fn publish_ready(
        &mut self,
        signature: u64,
        pane: usize,
        busy: bool,
        projection: &Arc<GroupProjection>,
    ) -> bool {
        self.publish_owner(signature, &GroupOwner::Main(pane), busy, projection)
    }

    fn publish_owner(
        &mut self,
        signature: u64,
        owner: &GroupOwner,
        busy: bool,
        projection: &Arc<GroupProjection>,
    ) -> bool {
        if busy {
            return false;
        }
        let Some(job) = self.jobs.get_mut(&signature) else {
            return false;
        };
        let Some(latest) = &job.latest else {
            return false;
        };
        let same_result = job
            .published
            .get(owner)
            .is_some_and(|published| Arc::ptr_eq(&published.groups, latest));
        let same_projection = job
            .published
            .get(owner)
            .and_then(|published| published.projection.upgrade())
            .is_some_and(|applied| Arc::ptr_eq(&applied, projection));
        if same_result && same_projection {
            return false;
        }
        job.published.insert(
            owner.clone(),
            Published {
                groups: latest.clone(),
                projection: Weak::new(),
            },
        );
        true
    }

    fn remember_projection(
        &mut self,
        signature: u64,
        pane: usize,
        projection: &Arc<GroupProjection>,
    ) {
        self.remember_owner(signature, &GroupOwner::Main(pane), projection);
    }

    fn remember_owner(
        &mut self,
        signature: u64,
        owner: &GroupOwner,
        projection: &Arc<GroupProjection>,
    ) {
        if let Some(job) = self.jobs.get_mut(&signature) {
            if let Some(published) = job.published.get_mut(owner) {
                published.projection = Arc::downgrade(projection);
            }
        }
    }

    fn needs_request(&self, signature: u64, now: Instant, ctx: &egui::Context) -> bool {
        let Some(job) = self.jobs.get(&signature) else {
            return true;
        };
        if job.in_flight {
            return false;
        }
        if let Some(retry) = job.retry_at {
            if retry <= now {
                return true;
            }
            ctx.request_repaint_after(retry.saturating_duration_since(now));
        }
        false
    }

    fn record_request(&mut self, signature: u64, cancelled: Arc<AtomicBool>) {
        if let Some(job) = self.jobs.get_mut(&signature) {
            job.cancelled.store(true, Ordering::Release);
            job.cancelled = cancelled;
            job.in_flight = true;
            job.retry_at = None;
            // Keep the last result and each pane's published snapshot until replacement is ready.
        } else {
            self.jobs.insert(signature, GroupJob::new(cancelled));
        }
    }
}

impl ColumnState {
    pub(crate) fn column_group_projection(
        &self,
        signature: u64,
        pane: usize,
        items: &[FileEntry],
        column: ColumnId,
        descending: bool,
    ) -> GroupProjection {
        self.grouping
            .jobs
            .get(&signature)
            .and_then(|job| job.published.get(&GroupOwner::Main(pane)))
            .map(|published| published.groups.project(items, column, descending))
            .unwrap_or_default()
    }

    pub(crate) fn remember_column_group_projection(
        &mut self,
        signature: u64,
        pane: usize,
        projection: &Arc<GroupProjection>,
    ) {
        self.grouping
            .remember_projection(signature, pane, projection);
    }
}

#[cfg(test)]
#[path = "grouping_tests.rs"]
mod tests;
