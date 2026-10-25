//! Shared column preferences and bounded, asynchronous display-property cache.
mod display_source;
mod display_worker;
mod grouping;
mod grouping_worker;
mod rank_order;
mod sorting;
mod sorting_worker;

use crate::domain::list_columns::{ColumnPreferences, PropertyId};
use crate::infrastructure::windows::column_properties::{
    self, PropertyColumn, PropertyDisplayValue,
};
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use lru::LruCache;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

const RETRY_DELAY: Duration = Duration::from_secs(5);

struct CachedValue {
    value: PropertyDisplayValue,
    retry_at: Option<Instant>,
}

impl CachedValue {
    fn new(text: Option<PropertyDisplayValue>, now: Instant) -> Self {
        let retry_at = text.is_none().then_some(now + RETRY_DELAY);
        Self {
            value: text.unwrap_or_else(|| PropertyDisplayValue::Text(String::new())),
            retry_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ValueKey {
    path: PathBuf,
    modified: u64,
    size: u64,
    created: Option<u64>,
    is_dir: bool,
    generation: usize,
    property: PropertyId,
}

impl ValueKey {
    fn new(
        item: &crate::domain::file_entry::FileEntry,
        generation: usize,
        property: PropertyId,
    ) -> Self {
        Self {
            path: item.path.clone(),
            modified: item.modified,
            size: item.size,
            created: item.created,
            is_dir: item.is_dir,
            generation,
            property,
        }
    }
}

pub struct ColumnState {
    pub(crate) content_revision: u64,
    pub preferences: ColumnPreferences,
    pub catalog: Vec<PropertyColumn>,
    pub catalog_loaded: bool,
    pub catalog_failed: bool,
    pub show_picker: bool,
    pub picker_owner: Option<egui::Id>,
    pub picker_search: String,
    sender: Option<Sender<display_worker::DisplayRequest>>,
    receiver: Receiver<(ValueKey, Option<PropertyDisplayValue>)>,
    catalog_receiver: Receiver<Result<Vec<PropertyColumn>, ()>>,
    values: LruCache<ValueKey, CachedValue>,
    pending: HashMap<ValueKey, Arc<AtomicBool>>,
    display_sources: display_source::DisplaySources,
    sorting: sorting::ColumnSorting,
    grouping: grouping::ColumnGrouping,
}

impl ColumnState {
    pub(crate) fn sort_pending(&self, pane: usize) -> bool {
        self.sorting.pending(pane)
    }

    pub fn new(preferences: ColumnPreferences) -> Self {
        let (sender, receiver) = display_worker::start();
        let (catalog_sender, catalog_receiver) = bounded(1);
        let started = crate::spawn_named("column-catalog", move || {
            let com = crate::infrastructure::windows::metadata::utils::ComGuard::new();
            let catalog = if com.is_some() {
                column_properties::catalog().map_err(|_| ())
            } else {
                Err(())
            };
            let _ = catalog_sender.send(catalog);
        })
        .is_ok();
        Self {
            content_revision: 0,
            preferences,
            catalog: Vec::new(),
            catalog_loaded: false,
            catalog_failed: !started,
            show_picker: false,
            picker_owner: None,
            picker_search: String::new(),
            sender,
            receiver,
            catalog_receiver,
            values: LruCache::new(NonZeroUsize::new(8192).unwrap()),
            pending: HashMap::new(),
            display_sources: Default::default(),
            sorting: sorting::ColumnSorting::new(),
            grouping: grouping::ColumnGrouping::new(),
        }
    }

    pub fn poll(&mut self) {
        if !self.catalog_loaded && !self.catalog_failed {
            match self.catalog_receiver.try_recv() {
                Ok(catalog) => {
                    self.content_revision = self.content_revision.wrapping_add(1);
                    self.catalog_loaded = true;
                    match catalog {
                        Ok(catalog) => self.catalog = catalog,
                        Err(()) => self.catalog_failed = true,
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.catalog_loaded = true;
                    self.catalog_failed = true;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        loop {
            match self.receiver.try_recv() {
                Ok((key, text)) => {
                    self.content_revision = self.content_revision.wrapping_add(1);
                    self.pending.remove(&key);
                    self.values.put(key, CachedValue::new(text, Instant::now()));
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.pending.clear();
                    self.sender = None;
                    break;
                }
            }
        }
    }

    pub(super) fn set_display_source(
        &mut self,
        pane: usize,
        source: Option<(usize, &std::path::Path)>,
    ) {
        self.display_sources.set(pane, source);
        self.pending
            .retain(|_, cancelled| !cancelled.load(Ordering::Acquire));
    }

    pub fn value(
        &mut self,
        ctx: &egui::Context,
        item: &crate::domain::file_entry::FileEntry,
        generation: usize,
        property: PropertyId,
    ) -> String {
        if property == column_properties::DATE_CREATED {
            if let Some(created) = item.created.filter(|&created| created > 0) {
                return crate::infrastructure::windows::format_date(created);
            }
        }
        let key = ValueKey::new(item, generation, property);
        if let Some(value) = self.values.get(&key) {
            if let Some(retry_at) = value.retry_at {
                let now = Instant::now();
                if now < retry_at {
                    ctx.request_repaint_after(retry_at - now);
                    return value.value.text();
                }
            } else {
                return value.value.text();
            }
        }
        self.request_values(ctx, item, generation, std::iter::once(property));
        String::new()
    }

    /// Queue a single file with all its enabled properties, instead of reopening it per cell.
    pub(crate) fn request_values(
        &mut self,
        ctx: &egui::Context,
        item: &crate::domain::file_entry::FileEntry,
        generation: usize,
        properties: impl IntoIterator<Item = PropertyId>,
    ) {
        let now = Instant::now();
        let mut keys = Vec::new();
        let mut seen = HashSet::new();
        for property in properties {
            if !seen.insert(property) {
                continue;
            }
            if property == column_properties::DATE_CREATED
                && item.created.is_some_and(|value| value > 0)
            {
                continue;
            }
            let key = ValueKey::new(item, generation, property);
            if self
                .pending
                .get(&key)
                .is_some_and(|cancelled| !cancelled.load(Ordering::Acquire))
            {
                continue;
            }
            if let Some(value) = self.values.peek(&key) {
                match value.retry_at {
                    None => continue,
                    Some(retry) if retry > now => {
                        ctx.request_repaint_after(retry - now);
                        continue;
                    }
                    _ => {}
                }
            }
            keys.push(key);
        }
        if keys.is_empty() {
            return;
        }
        if let Some(sender) = &self.sender {
            let request = display_worker::DisplayRequest {
                keys,
                ctx: ctx.clone(),
                cancelled: self.display_sources.current.clone(),
            };
            // Only successfully queued cells become pending, including when the queue is full.
            let pending = request.keys.clone();
            if sender.try_send(request).is_ok() {
                self.pending.extend(
                    pending
                        .into_iter()
                        .map(|key| (key, self.display_sources.current.clone())),
                );
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
    }

    /// Inspect existing metadata for layout measurement without scheduling I/O or changing LRU priority.
    pub(crate) fn cached_value(
        &self,
        item: &crate::domain::file_entry::FileEntry,
        generation: usize,
        property: PropertyId,
    ) -> Option<String> {
        if property == column_properties::DATE_CREATED {
            if let Some(created) = item.created.filter(|&created| created > 0) {
                return Some(crate::infrastructure::windows::format_date(created));
            }
        }
        self.values
            .peek(&ValueKey::new(item, generation, property))
            .filter(|value| value.retry_at.is_none())
            .map(|value| value.value.text())
    }
}

#[cfg(test)]
#[path = "column_state/tests.rs"]
mod tests;
