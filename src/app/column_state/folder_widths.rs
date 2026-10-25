//! Shared per-location manual widths, bounded hot cache, and debounced background persistence.
mod worker;
use crate::domain::{
    column_widths::{valid_width, WidthLocation},
    list_columns::ColumnId,
};
use crate::infrastructure::app_state_db::AppStateDb;
use eframe::egui;
use lru::LruCache;
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
use worker::{Request, Response, Worker};

const DEBOUNCE: Duration = Duration::from_millis(300);
const RETRY: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct Change {
    location: WidthLocation,
    column: ColumnId,
    width: f32,
    revision: u64,
}
#[derive(Default)]
struct Entry {
    values: HashMap<ColumnId, f32>,
    edited: HashSet<ColumnId>,
    loaded: bool,
    loading: bool,
    retry_at: Option<Instant>,
}

pub(crate) struct FolderWidths {
    db: Option<Arc<AppStateDb>>,
    worker: Option<Worker>,
    entries: LruCache<WidthLocation, Entry>,
    dirty: HashMap<(WidthLocation, ColumnId), Change>,
    in_flight: Option<Vec<Change>>,
    revision: u64,
    save_at: Option<Instant>,
    restart_at: Option<Instant>,
    context: Option<egui::Context>,
}

impl Default for FolderWidths {
    fn default() -> Self {
        Self {
            db: None,
            worker: None,
            entries: LruCache::new(std::num::NonZeroUsize::new(128).unwrap()),
            dirty: HashMap::new(),
            in_flight: None,
            revision: 0,
            save_at: None,
            restart_at: None,
            context: None,
        }
    }
}

impl FolderWidths {
    pub fn start(&mut self, db: Arc<AppStateDb>) {
        self.db.get_or_insert(db);
        self.ensure_worker();
    }

    pub fn poll_pending(&mut self) {
        if let Some(ctx) = self.context.clone() {
            self.tick(&ctx);
        }
    }

    fn ensure_worker(&mut self) {
        if self.worker.is_some() || self.restart_at.is_some_and(|retry| retry > Instant::now()) {
            return;
        }
        if let Some(db) = &self.db {
            self.worker = Worker::start(db.clone()).ok();
            if self.worker.is_none() {
                self.restart_at = Some(Instant::now() + RETRY);
            }
        }
    }

    fn entry(&mut self, location: &WidthLocation) -> &mut Entry {
        if !self.entries.contains(location) {
            let mut entry = Entry::default();
            for change in self
                .dirty
                .values()
                .filter(|change| &change.location == location)
            {
                entry.values.insert(change.column, change.width);
                entry.edited.insert(change.column);
            }
            self.entries.put(location.clone(), entry);
        }
        self.entries
            .get_mut(location)
            .expect("width cache entry exists")
    }

    pub fn frame(
        &mut self,
        location: &WidthLocation,
        defaults: &[(ColumnId, f32)],
        ctx: &egui::Context,
    ) -> Vec<(ColumnId, f32)> {
        self.tick(ctx);
        let entry = self.entry(location);
        let load = !entry.loaded
            && !entry.loading
            && entry.retry_at.is_none_or(|retry| retry <= Instant::now());
        if let Some(retry) = entry.retry_at {
            ctx.request_repaint_after(retry.saturating_duration_since(Instant::now()));
        }
        if load {
            if self.worker.as_ref().is_some_and(|worker| {
                worker
                    .requests
                    .try_send(Request::Load(location.clone(), ctx.clone()))
                    .is_ok()
            }) {
                self.entry(location).loading = true;
            } else {
                ctx.request_repaint_after(RETRY);
            }
        }
        let values = &self.entry(location).values;
        defaults
            .iter()
            .map(|&(id, width)| (id, values.get(&id).copied().unwrap_or(width)))
            .collect()
    }

    pub fn edit(
        &mut self,
        location: &WidthLocation,
        changes: &[(ColumnId, f32)],
        ctx: &egui::Context,
    ) {
        self.context = Some(ctx.clone());
        for &(column, width) in changes {
            if !valid_width(column, width) {
                continue;
            }
            let entry = self.entry(location);
            if entry.values.get(&column) == Some(&width) {
                continue;
            }
            entry.values.insert(column, width);
            entry.edited.insert(column);
            self.revision += 1;
            let change = Change {
                location: location.clone(),
                column,
                width,
                revision: self.revision,
            };
            self.dirty.insert((location.clone(), column), change);
            self.save_at = Some(Instant::now() + DEBOUNCE);
            ctx.request_repaint_after(DEBOUNCE);
        }
    }

    fn handle(&mut self, response: Response) -> bool {
        match response {
            Response::Loaded(location, result) => {
                if let Some(entry) = self.entries.get_mut(&location) {
                    entry.loading = false;
                    match result {
                        Ok(values) => {
                            for (id, width) in values {
                                if !entry.edited.contains(&id) {
                                    entry.values.insert(id, width);
                                }
                            }
                            entry.loaded = true;
                            entry.retry_at = None;
                        }
                        Err(()) => {
                            entry.retry_at = Some(Instant::now() + RETRY);
                        }
                    }
                }
            }
            Response::Saved(success) => {
                if let Some(changes) = self.in_flight.take() {
                    if success {
                        for change in changes {
                            let key = (change.location, change.column);
                            if self
                                .dirty
                                .get(&key)
                                .is_some_and(|current| current.revision == change.revision)
                            {
                                self.dirty.remove(&key);
                            }
                        }
                    } else {
                        self.save_at = Some(Instant::now() + RETRY);
                        log::warn!("Could not save folder column widths; retrying in background");
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        self.context = Some(ctx.clone());
        loop {
            let response = self
                .worker
                .as_ref()
                .and_then(|worker| worker.results.try_recv().ok());
            let Some(response) = response else {
                break;
            };
            self.handle(response);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.running.load(Ordering::Acquire))
        {
            self.worker = None;
            self.in_flight = None;
            for (_, entry) in self.entries.iter_mut() {
                entry.loading = false;
            }
            self.restart_at = Some(Instant::now() + RETRY);
        }
        self.ensure_worker();
        if let Some(deadline) = self.save_at {
            if deadline <= Instant::now() {
                self.submit();
            } else if !self.dirty.is_empty() {
                ctx.request_repaint_after(deadline.saturating_duration_since(Instant::now()));
            }
        }
        if !self.dirty.is_empty() && self.in_flight.is_none() {
            ctx.request_repaint_after(RETRY);
        }
    }

    fn submit(&mut self) {
        if self.in_flight.is_some() || self.dirty.is_empty() {
            return;
        }
        let Some(ctx) = &self.context else {
            return;
        };
        let changes: Vec<_> = self.dirty.values().cloned().collect();
        if self.worker.as_ref().is_some_and(|worker| {
            worker
                .requests
                .try_send(Request::Save(changes.clone(), ctx.clone()))
                .is_ok()
        }) {
            self.in_flight = Some(changes);
        }
    }

    /// Called only during shutdown/update: respect the caller's bounded final-save deadline.
    pub fn flush(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        self.ensure_worker();
        loop {
            self.submit();
            if self.dirty.is_empty() && self.in_flight.is_none() {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Some(response) = self
                .worker
                .as_ref()
                .and_then(|worker| worker.results.recv_timeout(remaining).ok())
            else {
                return false;
            };
            if !self.handle(response) {
                return false;
            }
        }
    }
}

#[cfg(test)]
#[path = "folder_widths/tests.rs"]
mod tests;
