use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::tag_id_from_view_path;
use crate::infrastructure::windows::RootAvailabilityCache;
use rustc_hash::FxHashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

/// State tracking for the focus-restore purge worker.
///
/// The previous implementation called `path.exists()` synchronously on the
/// UI thread for every item in every open tag view when the window regained
/// focus, which could block the UI for seconds on a cold NTFS cache (the same
/// case the rest of the tag-view performance work targets). This struct
/// replaces that synchronous scan with a single-shot background thread that
/// sends results back through the receiver drained by `process_purge_results`.
pub struct PurgeWorkerState {
    /// Coalesces multiple focus-restore events into a single running worker.
    /// While a scan is in flight, additional requests are dropped because the
    /// active scan already covers every item in every open tag view.
    pub running: Arc<AtomicBool>,
    /// Output channel for the worker. The sender is taken at spawn time and
    /// moved into the worker thread; the receiver stays here for the message
    /// handler to drain.
    pub receiver: std::sync::Mutex<Option<mpsc::Receiver<PurgeResult>>>,
}

pub struct PurgeResult {
    missing_paths: Vec<PathBuf>,
    unavailable_paths: Vec<PathBuf>,
}

impl PurgeWorkerState {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<PurgeResult>();
        // Drop the sender we kept here — the worker will install a fresh one.
        drop(sender);
        Self {
            running: Arc::new(AtomicBool::new(false)),
            receiver: std::sync::Mutex::new(Some(receiver)),
        }
    }
}

impl Default for PurgeWorkerState {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageViewerApp {
    /// Schedules an async purge of missing files from currently open tag
    /// views. Safe to call from any frame; coalesces concurrent requests.
    pub fn spawn_purge_missing_tag_views(&mut self) {
        process_purge_results(self);

        let Some(state_ref) = self.purge_worker_state.as_ref() else {
            return;
        };

        // Coalesce: if a worker is already running, do nothing. The
        // compare_exchange swaps the flag and is the single source of truth
        // for "is a scan in flight". Subsequent failures below MUST reset
        // the flag so the next focus-restore event can spawn again.
        if state_ref
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let paths = self.collect_active_tag_view_paths();
        if paths.is_empty() {
            state_ref.running.store(false, Ordering::Release);
            return;
        }

        // Build a fresh channel for this invocation. Pending results were
        // applied before acquiring the running flag above.
        let (tx, rx) = mpsc::channel::<PurgeResult>();
        if let Ok(mut guard) = state_ref.receiver.lock() {
            *guard = Some(rx);
        }

        let ui_ctx = self.ui_ctx.clone();
        let running_flag = state_ref.running.clone();

        let spawn_result = std::thread::Builder::new()
            .name("tag-view-purge".into())
            .spawn(move || {
                let mut root_availability = RootAvailabilityCache::default();
                let mut missing_candidates: Vec<PathBuf> = Vec::new();
                let mut unavailable_paths: Vec<PathBuf> = Vec::new();
                for path in paths {
                    if !root_availability.is_root_accessible(&path) {
                        unavailable_paths.push(path);
                    } else if !crate::infrastructure::onedrive::fast_path_exists(&path) {
                        missing_candidates.push(path);
                    }
                }

                if !missing_candidates.is_empty() || !unavailable_paths.is_empty() {
                    let _ = tx.send(PurgeResult {
                        missing_paths: missing_candidates,
                        unavailable_paths,
                    });
                    ui_ctx.request_repaint();
                }
                running_flag.store(false, Ordering::Release);
            });

        // If the spawn fails, release the running flag so the next call can
        // retry instead of being permanently locked out.
        if spawn_result.is_err() {
            state_ref.running.store(false, Ordering::Release);
            log::warn!("[TAGS] Failed to spawn tag-view purge worker");
        }
    }

    /// Collects the paths of items in all currently open tag views (active
    /// panel + inactive panel + tab snapshots for non-active tabs) so the
    /// purge worker can stat them off-thread. The caller has already filtered
    /// to tag views by checking `tag_id_from_view_path` on the view path, so
    /// the individual item paths here are real file paths (not virtual view
    /// paths).
    fn collect_active_tag_view_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut seen = FxHashSet::default();
        let mut push = |all_items: &std::sync::Arc<Vec<crate::domain::file_entry::FileEntry>>| {
            for item in all_items.iter() {
                if seen.insert(crate::domain::file_tag::normalize_tag_path_key(&item.path)) {
                    paths.push(item.path.clone());
                }
            }
        };

        if tag_id_from_view_path(&self.navigation_state.current_path).is_some() {
            push(&self.all_items);
        }

        if let Some(snapshot) = self.dual_panel_inactive_state.as_ref() {
            if tag_id_from_view_path(&snapshot.path).is_some() {
                push(&snapshot.all_items);
            }
        }

        let active_tab = self.tab_manager.active_tab;
        for (index, tab) in self.tab_manager.tabs.iter().enumerate() {
            if index == active_tab {
                continue;
            }
            if tag_id_from_view_path(&tab.path).is_some() {
                push(&tab.all_items);
            }
            if let Some(snapshot) = tab.dual_panel_inactive_state.as_ref() {
                if tag_id_from_view_path(&snapshot.path).is_some() {
                    push(&snapshot.all_items);
                }
            }
        }

        paths
    }
}

/// Drains any pending purge results from the focus-restore worker and applies
/// the reconciliation. Called from `process_incoming_messages` on the UI
/// thread.
pub fn process_purge_results(app: &mut ImageViewerApp) {
    let Some(state) = app.purge_worker_state.as_ref() else {
        return;
    };

    let mut missing_paths: Vec<PathBuf> = Vec::new();
    let mut unavailable_paths: Vec<PathBuf> = Vec::new();
    if let Ok(guard) = state.receiver.lock() {
        if let Some(rx) = guard.as_ref() {
            while let Ok(result) = rx.try_recv() {
                missing_paths.extend(result.missing_paths);
                unavailable_paths.extend(result.unavailable_paths);
            }
        }
    }

    // Results can arrive after a drive has been remounted. Revalidate before
    // pruning so stale worker output cannot hide newly available items.
    let mut current_roots = RootAvailabilityCache::default();
    unavailable_paths.retain(|path| !current_roots.is_root_accessible(path));
    missing_paths.retain(|path| {
        !current_roots.is_root_accessible(path)
            || !crate::infrastructure::onedrive::fast_path_exists(path)
    });
    unavailable_paths.extend(missing_paths);

    if !unavailable_paths.is_empty() {
        app.hide_unavailable_paths_from_tag_views(&unavailable_paths);
    }
}
