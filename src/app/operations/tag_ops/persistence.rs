//! FIFO assignment writes. UI state is projected immediately; failures reconcile
//! from a worker snapshot only when no newer projected mutation can be lost.
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::infrastructure::app_state_db::AppStateDb;
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::Duration;

pub(super) enum TagWrite {
    Assign(Vec<PathBuf>, Vec<i64>, Vec<FileEntry>),
    HealAuto {
        paths: Vec<PathBuf>,
        rule_tag: i64,
        ids: Vec<i64>,
    },
    Unassign(Vec<PathBuf>, Vec<i64>),
    Clear(Vec<PathBuf>),
    ClearCopied {
        paths: Vec<PathBuf>,
        expected_rows: usize,
    },
    Move(Vec<(PathBuf, PathBuf, i64)>),
    Reload,
}

struct ResultMessage {
    ticket: u64,
    failed: bool,
    snapshot: Option<FxHashMap<PathBuf, Vec<i64>>>,
    reload_failed: bool,
}

#[derive(Clone)]
pub(crate) struct TagWriteBarrier {
    state: Arc<(Mutex<u64>, Condvar)>,
    ticket: u64,
}

struct CompletionGuard {
    state: Arc<(Mutex<u64>, Condvar)>,
    ctx: eframe::egui::Context,
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        let (lock, ready) = &*self.state;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = u64::MAX;
        ready.notify_all();
        self.ctx.request_repaint();
    }
}

impl TagWriteBarrier {
    pub fn wait(&self, cancelled: impl Fn() -> bool) -> bool {
        let (lock, ready) = &*self.state;
        let mut completed = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *completed < self.ticket {
            if cancelled() {
                return false;
            }
            completed = ready
                .wait_timeout(completed, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        !cancelled()
    }
}

pub(crate) struct TagPersistence {
    sender: Option<mpsc::Sender<(u64, TagWrite)>>,
    receiver: mpsc::Receiver<ResultMessage>,
    worker: Option<std::thread::JoinHandle<()>>,
    state: Arc<(Mutex<u64>, Condvar)>,
    submitted: u64,
    acknowledged: u64,
    needs_reload: bool,
    retry_at: Option<std::time::Instant>,
}

impl TagPersistence {
    pub fn new(db: Arc<AppStateDb>, ctx: eframe::egui::Context) -> Self {
        let (sender, requests) = mpsc::channel::<(u64, TagWrite)>();
        let (results, receiver) = mpsc::channel();
        let state = Arc::new((Mutex::new(0), Condvar::new()));
        let completed = state.clone();
        let worker = std::thread::Builder::new()
            .name("tag-writes".into())
            .spawn(move || {
                let _completion = CompletionGuard {
                    state: completed.clone(),
                    ctx: ctx.clone(),
                };
                while let Ok((ticket, operation)) = requests.recv() {
                    let (success, reload) = execute(&db, operation);
                    let loaded = (!success || reload).then(|| db.try_get_all_tag_assignments());
                    let reload_failed = loaded.as_ref().is_some_and(Result::is_err);
                    let snapshot = loaded.and_then(Result::ok);
                    let (lock, ready) = &*completed;
                    *lock.lock().unwrap_or_else(|e| e.into_inner()) = ticket;
                    ready.notify_all();
                    let _ = results.send(ResultMessage {
                        ticket,
                        failed: !success || reload_failed,
                        snapshot,
                        reload_failed,
                    });
                    ctx.request_repaint();
                }
            })
            .ok();
        Self {
            sender: Some(sender),
            receiver,
            worker,
            state,
            submitted: 0,
            acknowledged: 0,
            needs_reload: false,
            retry_at: None,
        }
    }

    fn submit(&mut self, operation: TagWrite) -> bool {
        let ticket = self.submitted + 1;
        if self
            .sender
            .as_ref()
            .is_none_or(|sender| sender.send((ticket, operation)).is_err())
        {
            return false;
        }
        self.submitted = ticket;
        true
    }

    pub fn is_pending(&self) -> bool {
        self.acknowledged < self.submitted || self.needs_reload
    }

    pub fn barrier(&self) -> TagWriteBarrier {
        TagWriteBarrier {
            state: self.state.clone(),
            ticket: self.submitted,
        }
    }

    pub fn flush_for_shutdown(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for TagPersistence {
    fn drop(&mut self) {
        self.flush_for_shutdown();
    }
}

fn execute(db: &AppStateDb, operation: TagWrite) -> (bool, bool) {
    match operation {
        TagWrite::Assign(paths, ids, entries) => {
            let success = db.assign_tags_batch(&paths, &ids);
            if success && !entries.is_empty() {
                db.upsert_cached_file_entries(&entries);
            }
            (success, false)
        }
        TagWrite::Unassign(paths, ids) => (db.unassign_tags_batch(&paths, &ids), false),
        TagWrite::HealAuto {
            paths,
            rule_tag,
            ids,
        } => match db.heal_auto_tag_assignments(&paths, rule_tag, &ids) {
            Some(all_present) => (true, !all_present),
            None => (false, true),
        },
        TagWrite::Clear(paths) => (db.clear_tag_assignments_for_paths(&paths).is_some(), false),
        TagWrite::ClearCopied {
            paths,
            expected_rows,
        } => match db.clear_tag_assignments_for_paths(&paths) {
            Some(rows) => (true, rows != expected_rows),
            None => (false, true),
        },
        TagWrite::Move(moves) => (db.move_tag_assignments(&moves), false),
        TagWrite::Reload => (true, true),
    }
}

fn snapshot_is_current(ticket: u64, submitted: u64) -> bool {
    ticket == submitted
}

impl ImageViewerApp {
    pub(super) fn queue_tag_write(&mut self, operation: TagWrite) -> bool {
        if self.tag_persistence.submit(operation) {
            true
        } else {
            self.notifications
                .warning(rust_i18n::t!("tags.persistence_failed").to_string());
            false
        }
    }

    pub(super) fn poll_tag_writes(&mut self) {
        let mut snapshot = None;
        for _ in 0..64 {
            let result = match self.tag_persistence.receiver.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Disconnected)
                    if self.tag_persistence.acknowledged < self.tag_persistence.submitted =>
                {
                    self.tag_persistence =
                        TagPersistence::new(self.app_state_db.clone(), self.ui_ctx.clone());
                    self.notifications
                        .warning(rust_i18n::t!("tags.persistence_failed").to_string());
                    self.queue_tag_write(TagWrite::Reload);
                    break;
                }
                Err(_) => break,
            };
            self.tag_persistence.acknowledged = result.ticket;
            if result.failed {
                self.notifications
                    .warning(rust_i18n::t!("tags.persistence_failed").to_string());
            }
            if result.reload_failed {
                self.tag_persistence.needs_reload = true;
                self.tag_persistence.retry_at =
                    Some(std::time::Instant::now() + Duration::from_secs(3));
            }
            if let Some(loaded) = result.snapshot {
                if snapshot_is_current(result.ticket, self.tag_persistence.submitted) {
                    snapshot = Some(loaded);
                    self.tag_persistence.needs_reload = false;
                } else {
                    self.tag_persistence.needs_reload = true;
                }
            }
        }
        if let Some(loaded) = snapshot {
            self.set_tag_assignments(loaded);
            self.recompute_tag_counts_from_assignments();
            self.tag_updates
                .changed
                .extend(self.tag_definitions.keys().copied());
        } else if self.tag_persistence.needs_reload
            && self.tag_persistence.acknowledged == self.tag_persistence.submitted
            && self
                .tag_persistence
                .retry_at
                .is_none_or(|due| std::time::Instant::now() >= due)
        {
            self.tag_persistence.needs_reload = false;
            if !self.queue_tag_write(TagWrite::Reload) {
                self.tag_persistence.needs_reload = true;
                self.tag_persistence.retry_at =
                    Some(std::time::Instant::now() + Duration::from_secs(3));
            }
        }
        if self.tag_persistence.needs_reload {
            self.ui_ctx.request_repaint_after(Duration::from_secs(3));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_tag::TagColor;

    #[test]
    fn fifo_writes_and_shutdown_preserve_the_last_user_intent() {
        let db = Arc::new(AppStateDb::new_in_memory().unwrap());
        let tag = db.create_tag("Tag", TagColor::Blue, None).unwrap();
        let first = PathBuf::from(r"C:\Data\file.txt");
        let target = PathBuf::from(r"D:\Target\file.txt");
        let mut writes = TagPersistence::new(db.clone(), eframe::egui::Context::default());
        assert!(writes.submit(TagWrite::Assign(vec![first.clone()], vec![tag], Vec::new())));
        assert!(writes.submit(TagWrite::Clear(vec![first.clone()])));
        assert!(writes.submit(TagWrite::Assign(vec![first.clone()], vec![tag], Vec::new())));
        assert!(writes.submit(TagWrite::Move(vec![(first.clone(), target.clone(), tag)])));
        let barrier = writes.barrier();
        writes.flush_for_shutdown();
        assert!(barrier.wait(|| false));
        let snapshot = db.try_get_all_tag_assignments().unwrap();
        assert!(!snapshot.contains_key(&first));
        assert_eq!(snapshot.get(&target), Some(&vec![tag]));
    }

    #[test]
    fn failed_write_returns_saved_snapshot_instead_of_silent_optimistic_success() {
        let db = Arc::new(AppStateDb::new_in_memory().unwrap());
        let tag = db.create_tag("Tag", TagColor::Blue, None).unwrap();
        let path = PathBuf::from(r"C:\Data\file.txt");
        assert!(db.assign_tags_batch(std::slice::from_ref(&path), &[tag]));
        let mut writes = TagPersistence::new(db, eframe::egui::Context::default());
        assert!(writes.submit(TagWrite::Assign(Vec::new(), vec![tag], Vec::new())));
        let result = writes
            .receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(result.failed);
        assert!(!result.reload_failed);
        assert_eq!(result.snapshot.unwrap().get(&path), Some(&vec![tag]));
    }

    #[test]
    fn cancelled_background_reader_does_not_wait_for_a_pending_write() {
        let barrier = TagWriteBarrier {
            state: Arc::new((Mutex::new(0), Condvar::new())),
            ticket: 1,
        };
        assert!(!barrier.wait(|| true));
    }

    #[test]
    fn stale_recovery_never_overwrites_a_newer_projected_edit() {
        assert!(!snapshot_is_current(1, 2));
        assert!(snapshot_is_current(2, 2));
    }

    #[test]
    fn copied_destination_written_by_another_worker_requests_reconciliation() {
        let db = Arc::new(AppStateDb::new_in_memory().unwrap());
        let tag = db.create_tag("Tag", TagColor::Blue, None).unwrap();
        let path = PathBuf::from(r"C:\Data\file.txt");
        assert!(db.assign_tags_batch(std::slice::from_ref(&path), &[tag]));
        let mut writes = TagPersistence::new(db, eframe::egui::Context::default());
        assert!(writes.submit(TagWrite::ClearCopied {
            paths: vec![path],
            expected_rows: 0
        }));
        let result = writes
            .receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(!result.failed);
        assert!(result.snapshot.unwrap().is_empty());
    }
}
