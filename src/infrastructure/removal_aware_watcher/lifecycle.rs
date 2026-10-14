use crate::infrastructure::windows::OwnedHandle;
use notify::windows::{MetaEvent, ReadDirectoryChangesWatcher};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;

#[derive(Default)]
struct Registry {
    suspended: HashSet<char>,
    controls: HashMap<char, Vec<Weak<Control>>>,
    epochs: HashMap<char, u64>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

pub(super) struct NativeState {
    pub watcher: Option<ReadDirectoryChangesWatcher>,
    pub completions: crossbeam_channel::Receiver<MetaEvent>,
    pub paths: HashSet<std::path::PathBuf>,
    remaining: Option<usize>,
}

impl NativeState {
    pub fn new(
        watcher: ReadDirectoryChangesWatcher,
        completions: crossbeam_channel::Receiver<MetaEvent>,
    ) -> Self {
        Self {
            watcher: Some(watcher),
            completions,
            paths: HashSet::new(),
            remaining: None,
        }
    }

    fn stop(&mut self, deadline: Instant) -> bool {
        if let Some(watcher) = self.watcher.take() {
            self.remaining = Some(self.paths.len());
            drop(watcher);
        }
        let remaining = self.remaining.get_or_insert(0);
        while *remaining > 0 {
            match self.completions.recv_deadline(deadline) {
                Ok(MetaEvent::SingleWatchComplete) => *remaining -= 1,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        true
    }
}

pub(super) struct Control {
    pub native: Mutex<NativeState>,
    pub monitor: Mutex<Option<OwnedHandle>>,
}

impl Control {
    pub fn stop(&self, deadline: Instant) -> bool {
        let Some(mut native) = self.native.try_lock_until(deadline) else {
            return false;
        };
        self.monitor.lock().take();
        native.stop(deadline)
    }
    pub fn is_closed(&self) -> bool {
        self.native.lock().watcher.is_none()
    }
}

pub(super) fn register(letter: char, control: &Arc<Control>) {
    let mut registry = registry().lock();
    let controls = registry.controls.entry(letter).or_default();
    controls.retain(|entry| entry.strong_count() > 0);
    controls.push(Arc::downgrade(control));
}

pub(crate) fn is_suspended(letter: char) -> bool {
    registry()
        .lock()
        .suspended
        .contains(&letter.to_ascii_uppercase())
}

pub(super) fn suspend(letter: char, deadline: Instant) -> bool {
    let controls = {
        let mut registry = registry().lock();
        registry.suspended.insert(letter);
        registry
            .controls
            .get(&letter)
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>()
    };
    let mut released = true;
    for control in controls {
        released &= control.stop(deadline);
    }
    released
}

pub(super) fn resume(letter: char) {
    registry().lock().suspended.remove(&letter);
}

pub(super) fn epoch(letter: char) -> u64 {
    *registry().lock().epochs.get(&letter).unwrap_or(&0)
}

pub(super) fn arrived(letter: char) {
    let mut registry = registry().lock();
    if registry.suspended.remove(&letter) {
        *registry.epochs.entry(letter).or_default() += 1;
    }
}
