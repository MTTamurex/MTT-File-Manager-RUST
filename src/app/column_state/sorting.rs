//! Full-list property sorting, isolated from the bounded visible-cell cache.
use super::sorting_worker::{run, SortRequest, SortResult};
use crate::app::ImageViewerApp;
use crate::domain::file_entry::{FileEntry, SortMode};
use crate::domain::list_columns::ColumnId;
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use lru::LruCache;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct SortJob {
    cancelled: Arc<AtomicBool>,
    ranks: Option<HashMap<PathBuf, usize>>,
    applied: [std::sync::Weak<Vec<FileEntry>>; 2],
    retry_at: Option<std::time::Instant>,
}

impl Drop for SortJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

pub(super) struct ColumnSorting {
    sender: Option<Sender<SortRequest>>,
    receiver: Receiver<(u64, SortResult)>,
    jobs: LruCache<u64, SortJob>,
    fingerprints: LruCache<usize, (Arc<Vec<FileEntry>>, u64)>,
    current: [Option<u64>; 2],
}

impl ColumnSorting {
    fn set_current(&mut self, pane: usize, signature: Option<u64>) {
        if self.current[pane] == signature {
            return;
        }
        let old = std::mem::replace(&mut self.current[pane], signature);
        if let Some(old) = old {
            if !self.current.contains(&Some(old)) {
                self.jobs.pop(&old);
            }
        }
    }

    fn poll(&mut self) {
        loop {
            match self.receiver.try_recv() {
                Ok((signature, result)) => {
                    if let Some(job) = self.jobs.get_mut(&signature) {
                        job.ranks = Some(result.ranks);
                        job.retry_at = result
                            .deferred
                            .then_some(std::time::Instant::now() + super::RETRY_DELAY);
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    if self.sender.take().is_some() {
                        log::error!("The column sorting worker stopped");
                    }
                    self.set_current(0, None);
                    self.set_current(1, None);
                    break;
                }
            }
        }
    }

    fn apply_ready(
        &mut self,
        signature: u64,
        pane: usize,
        items: &mut Arc<Vec<FileEntry>>,
        busy: bool,
    ) -> bool {
        if busy {
            return false;
        }
        let Some(job) = self.jobs.get_mut(&signature) else {
            return false;
        };
        let Some(ranks) = &job.ranks else {
            return false;
        };
        if job.applied[pane]
            .upgrade()
            .is_some_and(|applied| Arc::ptr_eq(&applied, items))
        {
            return false;
        }
        super::rank_order::apply(Arc::make_mut(items).as_mut_slice(), ranks);
        job.applied[pane] = Arc::downgrade(items);
        true
    }

    pub(super) fn pending(&self, pane: usize) -> bool {
        self.sender.is_some()
            && self.current[pane].is_some_and(|signature| {
                self.jobs
                    .peek(&signature)
                    .is_some_and(|job| job.ranks.is_none())
            })
    }

    pub(super) fn new() -> Self {
        let (sender, requests) = bounded::<SortRequest>(2);
        let (results, receiver) = bounded(4);
        let started = crate::spawn_named("column-sort", move || {
            while let Ok(request) = requests.recv() {
                // A failed COM initialization still completes the job with deterministic name order.
                let ranks = run(&request).unwrap_or_default();
                if !request.cancelled.load(Ordering::Relaxed) {
                    if results.send((request.signature, ranks)).is_err() {
                        break;
                    }
                    request.ctx.request_repaint();
                }
            }
        })
        .map_err(|error| log::error!("Could not start the column sorting worker: {error}"))
        .is_ok();
        Self {
            sender: started.then_some(sender),
            receiver,
            jobs: LruCache::new(NonZeroUsize::new(4).unwrap()),
            fingerprints: LruCache::new(NonZeroUsize::new(4).unwrap()),
            current: [None, None],
        }
    }

    pub(super) fn fingerprint(&mut self, items: &Arc<Vec<FileEntry>>) -> u64 {
        let pointer = Arc::as_ptr(items) as usize;
        if let Some((_, fingerprint)) = self.fingerprints.get(&pointer) {
            return *fingerprint;
        }
        // Order independent: applying our own sort must not invalidate its result.
        let fingerprint = items.iter().fold(0, |sum: u64, item| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            item.path.hash(&mut hash);
            item.name.hash(&mut hash);
            item.is_dir.hash(&mut hash);
            item.modified.hash(&mut hash);
            item.size.hash(&mut hash);
            item.created.hash(&mut hash);
            item.recycle_original_path().hash(&mut hash);
            item.drive_info
                .as_ref()
                .map(|drive| (drive.total_space, drive.free_space))
                .hash(&mut hash);
            sum.wrapping_add(hash.finish())
        });
        self.fingerprints.put(pointer, (items.clone(), fingerprint));
        fingerprint
    }
}

impl ImageViewerApp {
    pub(crate) fn refresh_column_sort(&mut self, ctx: &egui::Context) {
        self.columns.poll();
        let pane = usize::from(
            matches!(
                self.dual_panel_active,
                crate::app::dual_panel::ActivePanel::Right
            ) ^ self.in_inactive_panel_context,
        );
        if !self.dual_panel_enabled {
            self.columns.set_display_source(1 - pane, None);
        }
        let source = (self.view_mode == crate::domain::file_entry::ViewMode::List).then_some((
            self.generation,
            std::path::Path::new(&self.navigation_state.current_path),
        ));
        self.columns.set_display_source(pane, source);
        let state = &mut self.columns.sorting;
        state.poll();
        if !self.dual_panel_enabled {
            state.set_current(1 - pane, None);
        }
        let SortMode::Column(column) = self.sort_mode else {
            state.set_current(pane, None);
            return;
        };
        if self.is_loading_folder || self.folder_load_error.is_some() {
            state.set_current(pane, None);
            return;
        }
        if state.sender.is_none() {
            return;
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        self.navigation_state.current_path.hash(&mut hash);
        self.generation.hash(&mut hash);
        state.fingerprint(&self.all_items).hash(&mut hash);
        self.all_items.len().hash(&mut hash);
        column.hash(&mut hash);
        self.sort_descending.hash(&mut hash);
        (self.folders_position as u8).hash(&mut hash);
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
        let signature = hash.finish();
        state.set_current(pane, Some(signature));
        if let Some(retry_at) = state.jobs.peek(&signature).and_then(|job| job.retry_at) {
            let now = std::time::Instant::now();
            if now >= retry_at {
                state.jobs.pop(&signature);
            } else {
                ctx.request_repaint_after(retry_at - now);
            }
        }
        if !state.jobs.contains(&signature) {
            let cancelled = Arc::new(AtomicBool::new(false));
            let request = SortRequest {
                signature,
                items: self.all_items.clone(),
                column,
                descending: self.sort_descending,
                folders: self.folders_position,
                tags: self.tag_assignments_normalized.clone(),
                definitions: self.tag_definitions.clone(),
                cancelled: cancelled.clone(),
                ctx: ctx.clone(),
            };
            let result = state
                .sender
                .as_ref()
                .expect("worker availability checked")
                .try_send(request);
            if matches!(
                result,
                Err(crossbeam_channel::TrySendError::Disconnected(_))
            ) {
                state.sender = None;
                log::error!("The column sorting worker is unavailable");
                return;
            }
            if result.is_ok() {
                state.jobs.put(
                    signature,
                    SortJob {
                        cancelled,
                        ranks: None,
                        applied: [std::sync::Weak::new(), std::sync::Weak::new()],
                        retry_at: None,
                    },
                );
            }
        }
        if !state.jobs.contains(&signature) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let anchor = self
            .selection_anchor
            .and_then(|index| self.items.get(index))
            .map(|item| item.path.clone());
        let busy = self.rectangle_selection_state.is_some()
            || self.is_item_dragging
            || self.external_drop_active
            || self.renaming_state.is_some()
            || self.context_menu.is_open;
        if !state.apply_ready(signature, pane, &mut self.items, busy) {
            return;
        }
        self.reconcile_visible_selection_index();
        self.selection_anchor =
            anchor.and_then(|path| self.items.iter().position(|item| item.path == path));
        self.rebuild_computer_view_indices();
        self.rebuild_group_projection();
    }
}

#[cfg(test)]
#[path = "sorting_tests.rs"]
mod tests;
