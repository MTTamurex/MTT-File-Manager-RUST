//! Optional deletion backend. No enumeration-only pass precedes deletion.
pub mod confirmation;
pub(crate) mod permanent;
pub(crate) mod recycle;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub(crate) type RemovalObserver = std::sync::Arc<dyn Fn(&Path) + Send + Sync>;

pub struct DeleteProgress {
    pub permanent_phase: AtomicBool,
    pub used_permanent_fallback: AtomicBool,
    pub confirmation: confirmation::Confirmation,
    pub permanently: bool,
    pub removed: AtomicU64,
    pub failures: AtomicU64,
    current: Mutex<Option<PathBuf>>,
}

impl DeleteProgress {
    pub fn new(permanently: bool) -> Self {
        Self {
            permanent_phase: AtomicBool::new(false),
            used_permanent_fallback: AtomicBool::new(false),
            confirmation: Default::default(),
            permanently,
            removed: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            current: Mutex::new(None),
        }
    }

    pub fn current(&self) -> Vec<PathBuf> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub(crate) fn set_current(&self, path: Option<&Path>) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = path.map(Path::to_path_buf);
    }

    pub(crate) fn removed(&self) {
        self.removed.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn failed(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }
}
