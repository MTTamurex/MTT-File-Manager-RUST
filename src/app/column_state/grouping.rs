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
use lru::LruCache;
use std::{
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::Instant,
};

struct GroupJob {
    cancelled: Arc<AtomicBool>,
    latest: Option<Arc<ColumnGroups>>,
    published: [Option<Arc<ColumnGroups>>; 2],
    projections: [Weak<GroupProjection>; 2],
    in_flight: bool,
    retry_at: Option<Instant>,
}

impl GroupJob {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            latest: None,
            published: [None, None],
            projections: [Weak::new(), Weak::new()],
            in_flight: true,
            retry_at: None,
        }
    }
}
impl Drop for GroupJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub(super) struct ColumnGrouping {
    sender: Option<Sender<GroupRequest>>,
    receiver: Receiver<(u64, ColumnGroups)>,
    jobs: LruCache<u64, GroupJob>,
    current: [Option<u64>; 2],
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
            jobs: LruCache::new(std::num::NonZeroUsize::new(4).unwrap()),
            current: [None, None],
        }
    }

    fn set_current(&mut self, pane: usize, signature: Option<u64>) -> bool {
        if self.current[pane] == signature {
            return false;
        }
        let old = std::mem::replace(&mut self.current[pane], signature);
        if let Some(old) = old {
            if !self.current.contains(&Some(old)) {
                self.jobs.pop(&old);
            }
        }
        true
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
                    self.set_current(0, None);
                    self.set_current(1, None);
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
        if busy {
            return false;
        }
        let Some(job) = self.jobs.get_mut(&signature) else {
            return false;
        };
        let Some(latest) = &job.latest else {
            return false;
        };
        let same_result = job.published[pane]
            .as_ref()
            .is_some_and(|published| Arc::ptr_eq(published, latest));
        let same_projection = job.projections[pane]
            .upgrade()
            .is_some_and(|applied| Arc::ptr_eq(&applied, projection));
        if same_result && same_projection {
            return false;
        }
        job.published[pane] = Some(latest.clone());
        true
    }

    fn remember_projection(
        &mut self,
        signature: u64,
        pane: usize,
        projection: &Arc<GroupProjection>,
    ) {
        if let Some(job) = self.jobs.get_mut(&signature) {
            if job.published[pane].is_some() {
                job.projections[pane] = Arc::downgrade(projection);
            }
        }
    }

    fn needs_request(&self, signature: u64, now: Instant, ctx: &egui::Context) -> bool {
        let Some(job) = self.jobs.peek(&signature) else {
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
            self.jobs.put(signature, GroupJob::new(cancelled));
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
            .peek(&signature)
            .and_then(|job| job.published[pane].as_ref())
            .map(|groups| groups.project(items, column, descending))
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

impl ImageViewerApp {
    pub(crate) fn column_group_signature(&mut self, column: ColumnId) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.navigation_state.current_path.hash(&mut hash);
        self.generation.hash(&mut hash);
        self.columns
            .sorting
            .fingerprint(&self.all_items)
            .hash(&mut hash);
        column.hash(&mut hash);
        rust_i18n::locale().to_string().hash(&mut hash);
        (crate::domain::date_format::date_format() as u8).hash(&mut hash);
        crate::application::grouping::local_grouping_day().hash(&mut hash);
        if column == ColumnId::Tags {
            self.tag_assignments_epoch.hash(&mut hash);
            for id in self.sorted_tag_ids.iter() {
                id.hash(&mut hash);
                self.tag_definitions
                    .get(id)
                    .map(|tag| &tag.name)
                    .hash(&mut hash);
            }
        }
        hash.finish()
    }

    pub(crate) fn refresh_column_grouping(&mut self, ctx: &egui::Context) {
        let pane = usize::from(
            matches!(
                self.dual_panel_active,
                crate::app::dual_panel::ActivePanel::Right
            ) ^ self.in_inactive_panel_context,
        );
        self.columns.grouping.poll();
        if !self.dual_panel_enabled {
            self.columns.grouping.set_current(1 - pane, None);
        }
        let GroupMode::Column(column) = self.group_mode else {
            self.columns.grouping.set_current(pane, None);
            return;
        };
        if self.navigation_state.is_computer_view
            || self.is_loading_folder
            || self.folder_load_error.is_some()
        {
            self.columns.grouping.set_current(pane, None);
            return;
        }
        let signature = self.column_group_signature(column);
        let state = &mut self.columns.grouping;
        let changed = state.set_current(pane, Some(signature));
        if state.needs_request(signature, Instant::now(), ctx) {
            if let Some(sender) = &state.sender {
                let cancelled = Arc::new(AtomicBool::new(false));
                let request = GroupRequest {
                    signature,
                    items: self.all_items.clone(),
                    column,
                    tags: self.tag_assignments_normalized.clone(),
                    definitions: self.tag_definitions.clone(),
                    cancelled: cancelled.clone(),
                    ctx: ctx.clone(),
                };
                if sender.try_send(request).is_ok() {
                    state.record_request(signature, cancelled);
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let busy = self.rectangle_selection_state.is_some()
            || self.is_item_dragging
            || self.external_drop_active
            || self.renaming_state.is_some()
            || self.context_menu.is_open;
        let publish = state.publish_ready(signature, pane, busy, &self.group_projection);
        if !busy && (changed || publish) {
            self.rebuild_group_projection();
        }
    }
}

#[cfg(test)]
#[path = "grouping_tests.rs"]
mod tests;
