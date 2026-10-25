//! Bounded file-level property reads. Shell interfaces never leave their COM thread.
use super::{PropertyDisplayValue, ValueKey};
use crate::infrastructure::windows::{
    column_properties::IndexedProperties, column_properties::PropertyReadError,
    column_properties::PropertyReader, metadata::utils::ComGuard,
};
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

const WORKERS: usize = 4;

pub(super) struct DisplayRequest {
    pub keys: Vec<ValueKey>,
    pub ctx: egui::Context,
    pub cancelled: Arc<AtomicBool>,
}

type DisplayResult = (ValueKey, Option<PropertyDisplayValue>);

pub(super) fn start() -> (Option<Sender<DisplayRequest>>, Receiver<DisplayResult>) {
    let (sender, requests) = bounded::<DisplayRequest>(128);
    let (results, receiver) = bounded(512);
    let mut started = false;
    for index in 0..WORKERS {
        let requests = requests.clone();
        let results = results.clone();
        started |= crate::spawn_named(&format!("column-properties-{index}"), move || {
            let Some(_com) = ComGuard::new() else {
                return;
            };
            let mut indexed = IndexedProperties::default();
            while let Ok(request) = requests.recv() {
                if !read_request(request, &results, &mut indexed) {
                    break;
                }
            }
        })
        .is_ok();
    }
    (started.then_some(sender), receiver)
}

fn read_request(
    request: DisplayRequest,
    results: &Sender<DisplayResult>,
    indexed: &mut IndexedProperties,
) -> bool {
    if request.cancelled.load(Ordering::Acquire) {
        return true;
    }
    let Some(first) = request.keys.first() else {
        return true;
    };
    let path = first.path.clone();
    let fast = PropertyReader::open_fast(&path).ok();
    let mut missing = Vec::new();
    for key in request.keys {
        if request.cancelled.load(Ordering::Acquire) {
            return true;
        }
        if let Some(value) = fast.as_ref().and_then(|reader| reader.read(key.property)) {
            let value = value.display_value(key.property);
            if results.send((key, Some(value))).is_err() {
                return false;
            }
            request.ctx.request_repaint();
        } else {
            missing.push(key);
        }
    }
    // A missing fast property says nothing about whether a property handler can supply it.
    if missing.is_empty() {
        return true;
    }
    if request.cancelled.load(Ordering::Acquire) {
        return true;
    }
    let first = &missing[0];
    let properties: Vec<_> = missing.iter().map(|key| key.property).collect();
    let mut cached = indexed.read(
        &path,
        first.modified,
        first.size,
        first.created,
        &properties,
    );
    if request.cancelled.load(Ordering::Acquire) {
        return true;
    }
    let mut uncached = Vec::new();
    for key in missing {
        if let Some(value) = cached.remove(&key.property) {
            if results.send((key, Some(value))).is_err() {
                return false;
            }
            request.ctx.request_repaint();
        } else {
            uncached.push(key);
        }
    }
    if uncached.is_empty() || request.cancelled.load(Ordering::Acquire) {
        return true;
    }
    let reader = PropertyReader::open(&path);
    let deferred = matches!(reader, Err(PropertyReadError::Deferred));
    for key in uncached {
        if request.cancelled.load(Ordering::Acquire) {
            return true;
        }
        let value = reader
            .as_ref()
            .ok()
            .and_then(|reader| reader.read(key.property))
            .map(|value| value.display_value(key.property))
            .unwrap_or_else(|| PropertyDisplayValue::Text(String::new()));
        if request.cancelled.load(Ordering::Acquire) {
            return true;
        }
        if results.send((key, (!deferred).then_some(value))).is_err() {
            return false;
        }
        request.ctx.request_repaint();
    }
    true
}

#[cfg(test)]
#[path = "display_worker_tests.rs"]
mod tests;
