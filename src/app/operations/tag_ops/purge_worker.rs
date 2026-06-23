use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::tag_id_from_view_path;
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
    pub receiver: std::sync::Mutex<Option<mpsc::Receiver<Vec<PathBuf>>>>,
}

impl PurgeWorkerState {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<Vec<PathBuf>>();
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
        let Some(state_ref) = self.purge_worker_state.as_ref() else {
            return;
        };

        // Drop any previous results that the UI thread did not consume.
        if let Ok(guard) = state_ref.receiver.lock() {
            if let Some(rx) = guard.as_ref() {
                while rx.try_recv().is_ok() {}
            }
        }

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

        // Build a fresh channel for this invocation. The previous receiver
        // is replaced (any unreceived results were discarded above).
        let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
        if let Ok(mut guard) = state_ref.receiver.lock() {
            *guard = Some(rx);
        }

        let ui_ctx = self.ui_ctx.clone();
        let running_flag = state_ref.running.clone();

        let spawn_result = std::thread::Builder::new()
            .name("tag-view-purge".into())
            .spawn(move || {
                let mut missing: Vec<PathBuf> = Vec::new();
                for path in paths {
                    if !path.exists() {
                        missing.push(path);
                    }
                }

                if !missing.is_empty() {
                    let _ = tx.send(missing);
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
        let mut push =
            |all_items: &std::sync::Arc<Vec<crate::domain::file_entry::FileEntry>>| {
                for item in all_items.iter() {
                    paths.push(item.path.clone());
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

    let mut collected: Vec<PathBuf> = Vec::new();
    if let Ok(guard) = state.receiver.lock() {
        if let Some(rx) = guard.as_ref() {
            while let Ok(batch) = rx.try_recv() {
                collected.extend(batch);
            }
        }
    }

    if collected.is_empty() {
        return;
    }

    app.reconcile_garbage_collected_tag_assignments(&collected);
}
