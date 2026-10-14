//! Native directory watchers that release their handles before PnP removal.
mod lifecycle;
mod notifications;
mod retired;
pub(crate) use lifecycle::is_suspended;
use notify::{EventHandler, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Group {
    registration: Option<notifications::Registration>,
    control: Arc<lifecycle::Control>,
}
impl Drop for Group {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            if registration.can_release() {
                drop(registration);
            } else {
                retired::retain_until_outcome(registration);
            }
        }
        self.control.stop(Instant::now() + Duration::from_secs(4));
    }
}

pub struct RemovalAwareWatcher {
    handler: Arc<Mutex<dyn EventHandler>>,
    groups: HashMap<Option<char>, Group>,
}

impl RemovalAwareWatcher {
    pub fn new(handler: impl EventHandler) -> notify::Result<Self> {
        Ok(Self {
            handler: Arc::new(Mutex::new(handler)),
            groups: HashMap::new(),
        })
    }

    pub fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        let letter = crate::infrastructure::windows::extract_drive_letter(path);
        if letter.is_some_and(is_suspended) {
            return Err(notify::Error::generic("Volume monitoring suspended"));
        }
        if !self.groups.contains_key(&letter) {
            let (tx, rx) = crossbeam_channel::unbounded();
            let watcher =
                notify::windows::ReadDirectoryChangesWatcher::create(self.handler.clone(), tx)?;
            let control = Arc::new(lifecycle::Control {
                native: parking_lot::Mutex::new(lifecycle::NativeState::new(watcher, rx)),
                monitor: parking_lot::Mutex::new(None),
            });
            let mut group = Group {
                registration: None,
                control: control.clone(),
            };
            if let Some(letter) = letter {
                lifecycle::register(letter, &control);
                group.registration = notifications::Registration::new(letter, control)
                    .map_err(|error| notify::Error::generic(&error))?;
            }
            self.groups.insert(letter, group);
        }
        let mut native = self.groups[&letter].control.native.lock();
        if letter.is_some_and(is_suspended) {
            return Err(notify::Error::generic("Volume monitoring suspended"));
        }
        if native.paths.contains(path) {
            return Ok(());
        }
        let watcher = native
            .watcher
            .as_mut()
            .ok_or_else(|| notify::Error::generic("Watcher released for volume removal"))?;
        watcher.watch(path, mode)?;
        native.paths.insert(PathBuf::from(path));
        Ok(())
    }

    pub fn needs_refresh(&self) -> bool {
        self.groups
            .iter()
            .any(|(letter, group)| !letter.is_some_and(is_suspended) && group.control.is_closed())
    }
}

pub(crate) fn volume_arrived(letter: char) {
    lifecycle::arrived(letter.to_ascii_uppercase());
}

pub(crate) fn path_is_suspended(path: &Path) -> bool {
    crate::infrastructure::windows::extract_drive_letter(path).is_some_and(is_suspended)
}

#[cfg(test)]
mod tests;
