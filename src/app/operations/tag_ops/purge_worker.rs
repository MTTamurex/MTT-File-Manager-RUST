use crate::app::state::ImageViewerApp;
use crate::domain::special_paths::tag_id_from_view_path;
use crate::infrastructure::io_priority::{IOPriority, ThreadPriorityGuard};
use crate::infrastructure::windows::RootAvailabilityCache;
use eframe::egui;
use rustc_hash::FxHashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    pub running: Arc<AtomicBool>,
    /// Set when a focus-restore request arrives while the scan is running. The
    /// next idle transition starts a fresh snapshot instead of dropping it.
    pub rescan_requested: Arc<AtomicBool>,
    /// Identifies the latest requested snapshot so a result from before a
    /// remount or view change cannot be applied to the new state.
    latest_request_id: Arc<AtomicU64>,
    /// Stable output channel shared by every worker invocation. Keeping the
    /// receiver in place avoids replacing a channel while a result is in flight.
    sender: mpsc::Sender<PurgeResult>,
    pub receiver: std::sync::Mutex<mpsc::Receiver<PurgeResult>>,
}

pub struct PurgeResult {
    request_id: u64,
    missing_paths: Vec<PathBuf>,
    unavailable_paths: Vec<PathBuf>,
}

fn is_current_request(result_request_id: u64, latest_request_id: u64) -> bool {
    result_request_id == latest_request_id
}

impl PurgeWorkerState {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<PurgeResult>();
        Self {
            running: Arc::new(AtomicBool::new(false)),
            rescan_requested: Arc::new(AtomicBool::new(false)),
            latest_request_id: Arc::new(AtomicU64::new(0)),
            sender,
            receiver: std::sync::Mutex::new(receiver),
        }
    }

    fn request_rescan(&self) {
        self.latest_request_id.fetch_add(1, Ordering::AcqRel);
        self.rescan_requested.store(true, Ordering::Release);
    }

    fn latest_request_id(&self) -> u64 {
        self.latest_request_id.load(Ordering::Acquire)
    }

    fn take_rescan_request_if_idle(&self) -> bool {
        if self.running.load(Ordering::Acquire) {
            return false;
        }
        self.rescan_requested.swap(false, Ordering::AcqRel)
    }
}

struct PurgeWorkerGuard {
    running: Arc<AtomicBool>,
    rescan_requested: Arc<AtomicBool>,
    ui_ctx: egui::Context,
}

impl Drop for PurgeWorkerGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        if self.rescan_requested.load(Ordering::Acquire) {
            self.ui_ctx.request_repaint();
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
        if let Some(state_ref) = self.purge_worker_state.as_ref() {
            state_ref.request_rescan();
        }
        process_purge_results(self);

        self.try_start_purge_missing_tag_views();
    }

    fn try_start_purge_missing_tag_views(&mut self) {
        let Some(state_ref) = self.purge_worker_state.as_ref() else {
            return;
        };

        if !state_ref.take_rescan_request_if_idle() {
            return;
        }

        // The running flag is the single source of truth for a scan in flight.
        // If another caller won the race, leave the request queued for it.
        if state_ref
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            state_ref.request_rescan();
            return;
        }
        let request_id = state_ref.latest_request_id();

        let paths = self.collect_active_tag_view_paths();
        if paths.is_empty() {
            state_ref.running.store(false, Ordering::Release);
            return;
        }

        let tx = state_ref.sender.clone();
        let ui_ctx = self.ui_ctx.clone();
        let worker_guard = PurgeWorkerGuard {
            running: state_ref.running.clone(),
            rescan_requested: state_ref.rescan_requested.clone(),
            ui_ctx: ui_ctx.clone(),
        };

        let spawn_result = std::thread::Builder::new()
            .name("tag-view-purge".into())
            .spawn(move || {
                let _worker_guard = worker_guard;
                // Focus-restore reconciliation is maintenance work and can
                // touch every item in every open tag view.
                let _priority_guard = ThreadPriorityGuard::new(IOPriority::Background);
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

                let has_result = !missing_candidates.is_empty() || !unavailable_paths.is_empty();
                if has_result {
                    let _ = tx.send(PurgeResult {
                        request_id,
                        missing_paths: missing_candidates,
                        unavailable_paths,
                    });
                }
                if has_result {
                    ui_ctx.request_repaint();
                }
            });

        // If the spawn fails, release the running flag and retain the request
        // so the next repaint can retry instead of dropping the scan.
        if spawn_result.is_err() {
            state_ref.running.store(false, Ordering::Release);
            state_ref.request_rescan();
            self.ui_ctx.request_repaint();
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
    let latest_request_id = state.latest_request_id();
    if let Ok(receiver) = state.receiver.lock() {
        while let Ok(result) = receiver.try_recv() {
            if !is_current_request(result.request_id, latest_request_id) {
                continue;
            }
            missing_paths.extend(result.missing_paths);
            unavailable_paths.extend(result.unavailable_paths);
        }
    }

    // The worker already performed the filesystem probes. Request IDs prevent
    // a result from an older snapshot from hiding paths after a remount or
    // view change, so no probe is needed on the UI thread.
    unavailable_paths.extend(missing_paths);

    if !unavailable_paths.is_empty() {
        app.queue_tag_view_hides_for_all_views(&unavailable_paths);
    }

    let follow_up_needed = app.purge_worker_state.as_ref().is_some_and(|state| {
        !state.running.load(Ordering::Acquire) && state.rescan_requested.load(Ordering::Acquire)
    });
    if follow_up_needed {
        app.try_start_purge_missing_tag_views();
    }
}

#[cfg(test)]
mod tests {
    use super::{is_current_request, PurgeWorkerState};
    use std::sync::atomic::Ordering;

    #[test]
    fn rescan_request_is_retained_while_worker_is_running() {
        let state = PurgeWorkerState::new();
        state.running.store(true, Ordering::Release);
        state.request_rescan();

        assert!(!state.take_rescan_request_if_idle());
        assert!(state.rescan_requested.load(Ordering::Acquire));
    }

    #[test]
    fn idle_rescan_request_is_consumed_once() {
        let state = PurgeWorkerState::new();
        state.request_rescan();

        assert!(state.take_rescan_request_if_idle());
        assert!(!state.take_rescan_request_if_idle());
    }

    #[test]
    fn each_rescan_request_gets_a_new_result_identity() {
        let state = PurgeWorkerState::new();

        state.request_rescan();
        let first = state.latest_request_id();
        state.request_rescan();
        let second = state.latest_request_id();

        assert_ne!(first, second);
        assert!(is_current_request(second, second));
        assert!(!is_current_request(first, second));
    }

    #[test]
    fn result_channel_remains_available_between_runs() {
        let state = PurgeWorkerState::new();
        state
            .sender
            .send(super::PurgeResult {
                request_id: 0,
                missing_paths: Vec::new(),
                unavailable_paths: Vec::new(),
            })
            .expect("stable purge channel");

        let receiver = state.receiver.lock().expect("receiver lock");
        assert!(receiver.try_recv().is_ok());
    }
}
