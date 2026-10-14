//! Nonblocking, periodically refreshed Google Drive overlay cache.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Sender};
use eframe::egui;
use lru::LruCache;
use parking_lot::Mutex;

use super::google_drive_shell;
use crate::domain::file_entry::SyncStatus;

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

struct CachedStatus {
    status: SyncStatus,
    checked_at: Instant,
}

struct State {
    entries: LruCache<PathBuf, CachedStatus>,
    pending: HashSet<PathBuf>,
}

struct Request {
    path: PathBuf,
    ctx: egui::Context,
}

struct Worker {
    state: Arc<Mutex<State>>,
    sender: Sender<Request>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();

fn worker() -> &'static Worker {
    WORKER.get_or_init(|| {
        let state = Arc::new(Mutex::new(State {
            entries: LruCache::new(NonZeroUsize::new(16384).unwrap()),
            pending: HashSet::new(),
        }));
        let (sender, receiver) = bounded::<Request>(256);
        let shared = Arc::clone(&state);
        let result = std::thread::Builder::new()
            .name("google-drive-status".into())
            .spawn(move || {
                let com = crate::infrastructure::windows::ComScope::sta();
                let mut overlays = if com.is_initialized() {
                    google_drive_shell::load_overlays()
                } else {
                    Vec::new()
                };
                let mut loaded_at = Instant::now();
                while let Ok(request) = receiver.recv() {
                    // Retry registrations after Drive starts or updates during this session.
                    if com.is_initialized() && loaded_at.elapsed() >= Duration::from_secs(60) {
                        overlays = google_drive_shell::load_overlays();
                        loaded_at = Instant::now();
                    }
                    let status = google_drive_shell::query_status(&overlays, &request.path)
                        .unwrap_or(SyncStatus::None);
                    {
                        let mut state = shared.lock();
                        state.entries.put(
                            request.path.clone(),
                            CachedStatus {
                                status,
                                checked_at: Instant::now(),
                            },
                        );
                        state.pending.remove(&request.path);
                    }
                    request.ctx.request_repaint();
                }
            });
        if let Err(error) = result {
            log::warn!("[GoogleDrive] Could not start sync-status worker: {error}");
        }
        Worker { state, sender }
    })
}

/// Returns the last known badge and queues stale/missing results. No Shell or
/// filesystem calls occur on the calling (UI) thread.
pub(super) fn cached_status(path: &Path, ctx: &egui::Context) -> SyncStatus {
    let worker = worker();
    let mut state = worker.state.lock();
    let cached = state.entries.get(path);
    let status = cached.map_or(SyncStatus::None, |entry| entry.status);
    let stale = cached.is_none_or(|entry| entry.checked_at.elapsed() >= REFRESH_INTERVAL);
    if stale && !state.pending.contains(path) {
        let request = Request {
            path: path.to_path_buf(),
            ctx: ctx.clone(),
        };
        if worker.sender.try_send(request).is_ok() {
            state.pending.insert(path.to_path_buf());
        }
    }
    // Also covers a temporarily full queue and changes not reported by a watcher.
    ctx.request_repaint_after(Duration::from_secs(1));
    status
}
