//! Count actual completed removals, not only S_OK or attempted Shell work units.
use super::*;
use crate::infrastructure::delete_engine::{DeleteProgress, RemovalObserver};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

#[derive(Default)]
struct State {
    originals: HashMap<usize, PathBuf>,
    candidates: HashMap<PathBuf, bool>,
    confirmed: HashSet<PathBuf>,
}

#[derive(Default)]
pub(super) struct Completion(Mutex<State>);

impl Completion {
    pub(super) fn begin(&self, key: Option<usize>, path: Option<PathBuf>) {
        if let (Some(key), Some(path)) = (key, path) {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .originals
                .insert(key, path);
        }
    }

    pub(super) fn record(
        &self,
        key: Option<usize>,
        path: Option<PathBuf>,
        result: HRESULT,
        recycled: bool,
        progress: &DeleteProgress,
        observer: &RemovalObserver,
    ) {
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let original = key.and_then(|key| state.originals.remove(&key)).or(path);
        // Positive HRESULTs are not all completions: S_FALSE and copy-engine
        // pending/skip/retry statuses must never inflate the counters.
        if result.is_ok()
            && ![
                windows::Win32::Foundation::S_FALSE,
                COPYENGINE_S_PENDING,
                COPYENGINE_S_PENDING_DELETE,
                COPYENGINE_S_USER_IGNORED,
                COPYENGINE_S_USER_RETRY,
                COPYENGINE_S_NOT_HANDLED,
                COPYENGINE_S_PROGRESS_PAUSE,
                COPYENGINE_S_CLOSE_PROGRAM,
            ]
            .contains(&result)
        {
            if let Some(path) = original {
                if !state.confirmed.contains(&path) {
                    let recycled = *state.candidates.entry(path.clone()).or_insert(recycled);
                    if Self::confirm_path(&path, recycled, &mut state.confirmed, progress, observer)
                    {
                        state.candidates.remove(&path);
                    }
                }
            }
        }
    }

    pub(super) fn flush(&self, progress: &DeleteProgress, observer: &RemovalObserver) {
        Self::confirm(
            &mut self.0.lock().unwrap_or_else(|e| e.into_inner()),
            progress,
            observer,
        );
    }

    fn confirm(state: &mut State, progress: &DeleteProgress, observer: &RemovalObserver) {
        let State {
            candidates,
            confirmed,
            ..
        } = state;
        // Deferred candidates are revisited once after PerformOperations, not
        // rescanned on every callback when a large batch contains pending paths.
        candidates.retain(|path, recycled| {
            !Self::confirm_path(path, *recycled, confirmed, progress, observer)
        });
    }

    fn confirm_path(
        path: &Path,
        recycled: bool,
        confirmed: &mut HashSet<PathBuf>,
        progress: &DeleteProgress,
        observer: &RemovalObserver,
    ) -> bool {
        if matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        {
            if confirmed.insert(path.to_owned()) {
                if recycled {
                    progress.recycled();
                } else {
                    progress.removed();
                }
                observer(path);
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests;
