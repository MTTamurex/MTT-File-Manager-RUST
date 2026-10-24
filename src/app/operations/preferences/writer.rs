//! One in-flight transaction plus one replaceable full snapshot. No SQLite work
//! or database locks run on the submitting thread, including the final flush.
use crate::infrastructure::app_state_db::AppStateDb;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Snapshot = (u64, Vec<(&'static str, String)>, Instant);

pub(crate) struct PreferenceWriter {
    requests: Sender<Snapshot>,
    replace: Receiver<Snapshot>,
    wake: Receiver<()>,
    persisted: Arc<AtomicU64>,
    failed: Arc<AtomicU64>,
    reported_failure: AtomicU64,
    running: Arc<AtomicBool>,
    next: u64,
}

impl PreferenceWriter {
    pub(crate) fn new(db: Arc<AppStateDb>) -> std::io::Result<Self> {
        let (requests, rx) = bounded::<Snapshot>(1);
        let replace = rx.clone();
        let (done, wake) = bounded(1);
        let persisted = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicU64::new(0));
        let success = persisted.clone();
        let failure = failed.clone();
        let running = Arc::new(AtomicBool::new(true));
        let lifetime = WorkerLifetime(running.clone());
        std::thread::Builder::new()
            .name("preferences-writer".into())
            .spawn(move || {
                let _lifetime = lifetime;
                while let Ok((id, snapshot, queued)) = rx.recv() {
                    let started = Instant::now();
                    let queue_wait = started.saturating_duration_since(queued);
                    let result = db.set_preferences_batch(&snapshot);
                    if result.is_ok() {
                        success.store(id, Ordering::Release);
                    } else {
                        failure.store(id, Ordering::Release);
                    }
                    let _ = done.try_send(());
                    crate::infrastructure::diagnostic_logger::diag_info(
                        "preferences",
                        "write_completed",
                        &[
                            crate::infrastructure::diagnostic_logger::field_u64("request_id", id),
                            crate::infrastructure::diagnostic_logger::field_bool(
                                "success",
                                result.is_ok(),
                            ),
                            crate::infrastructure::diagnostic_logger::field_duration_ms(
                                "queue_wait",
                                queue_wait,
                            ),
                            crate::infrastructure::diagnostic_logger::field_duration_ms(
                                "duration",
                                started.elapsed(),
                            ),
                        ],
                    );
                }
            })?;
        Ok(Self {
            requests,
            replace,
            wake,
            persisted,
            failed,
            reported_failure: AtomicU64::new(0),
            running,
            next: 0,
        })
    }

    pub(crate) fn submit(&mut self, snapshot: Vec<(&'static str, String)>) -> Option<u64> {
        if !self.is_running() {
            return None;
        }
        self.next += 1;
        let id = self.next;
        let request = (id, snapshot, Instant::now());
        match self.requests.try_send(request) {
            Ok(()) => Some(id),
            Err(crossbeam_channel::TrySendError::Full(mut request)) => {
                if let Ok((_, previous, _)) = self.replace.try_recv() {
                    merge_omitted_preferences(previous, &mut request.1);
                }
                self.requests.try_send(request).ok().map(|_| id)
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => None,
        }
    }

    pub(crate) fn take_failure(&self) -> bool {
        let failed = self.failed.load(Ordering::Acquire);
        failed > self.persisted.load(Ordering::Acquire)
            && failed > self.reported_failure.fetch_max(failed, Ordering::AcqRel)
    }

    pub(crate) fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub(crate) fn wait(&self, id: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.persisted.load(Ordering::Acquire) >= id {
                return true;
            }
            if self.failed.load(Ordering::Acquire) >= id {
                return false;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || self.wake.recv_timeout(remaining).is_err() {
                return false;
            }
        }
    }
}

struct WorkerLifetime(Arc<AtomicBool>);
impl Drop for WorkerLifetime {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

// Snapshots can omit keys (e.g. last_folder in a virtual view). A newer
// snapshot supersedes matching keys, not updates to keys it does not contain.
fn merge_omitted_preferences(
    previous: Vec<(&'static str, String)>,
    latest: &mut Vec<(&'static str, String)>,
) {
    let keys: std::collections::HashSet<_> = latest.iter().map(|(key, _)| *key).collect();
    latest.extend(previous.into_iter().filter(|(key, _)| !keys.contains(key)));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_database_does_not_block_submission_or_final_deadline() {
        let db = Arc::new(AppStateDb::new_in_memory().unwrap());
        let mut writer = PreferenceWriter::new(db.clone()).unwrap();
        // The in-memory reader shares the writer lock. A long SQLite query
        // is unnecessary: use the same process-local lock deterministically.
        let guard = db.test_hold_writer();
        let start = Instant::now();
        let old = writer.submit(vec![("key", "old".into())]).unwrap();
        let latest = writer.submit(vec![("key", "latest".into())]).unwrap();
        assert!(latest > old);
        assert!(!writer.wait(latest, Duration::from_millis(20)));
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(guard);
        assert!(writer.wait(latest, Duration::from_secs(2)));
        assert_eq!(db.get_preference("key").as_deref(), Some("latest"));
    }

    #[test]
    fn failed_transaction_is_reported_and_a_new_snapshot_can_recover() {
        let db = Arc::new(AppStateDb::new_in_memory().unwrap());
        db.test_hold_writer().execute_batch("CREATE TRIGGER fail_preference BEFORE INSERT ON user_preferences BEGIN SELECT RAISE(FAIL, 'test failure'); END;").unwrap();
        let mut writer = PreferenceWriter::new(db.clone()).unwrap();
        let failed = writer.submit(vec![("key", "old".into())]).unwrap();
        assert!(!writer.wait(failed, Duration::from_secs(2)));
        assert!(writer.take_failure());
        assert!(!writer.take_failure());
        // Observing an error must not erase the completion outcome used by wait.
        assert!(!writer.wait(failed, Duration::from_secs(2)));
        db.test_hold_writer()
            .execute_batch("DROP TRIGGER fail_preference;")
            .unwrap();
        let latest = writer.submit(vec![("key", "latest".into())]).unwrap();
        assert!(writer.wait(latest, Duration::from_secs(2)));
        assert_eq!(db.get_preference("key").as_deref(), Some("latest"));
    }

    #[test]
    fn coalescing_preserves_omitted_keys_and_uses_latest_matching_values() {
        let mut latest = vec![("view_mode", "virtual".to_string())];
        merge_omitted_preferences(
            vec![
                ("view_mode", "list".into()),
                ("last_folder", "folder".into()),
            ],
            &mut latest,
        );
        assert_eq!(
            latest,
            vec![
                ("view_mode", "virtual".into()),
                ("last_folder", "folder".into())
            ]
        );
    }

    #[test]
    fn stopped_worker_is_not_mistaken_for_a_live_channel() {
        let db = Arc::new(AppStateDb::new_in_memory().unwrap());
        let mut writer = PreferenceWriter::new(db).unwrap();
        writer.running.store(false, Ordering::Release);
        assert!(writer.submit(vec![("key", "value".into())]).is_none());
    }

    #[test]
    fn full_request_slot_merges_sparse_snapshots_without_growing_the_queue() {
        let (requests, rx) = bounded(1);
        let (_done, wake) = bounded(1);
        // Deliberately do not start a consumer, so every replacement takes the
        // Full branch deterministically rather than racing a worker thread.
        let mut writer = PreferenceWriter {
            requests,
            replace: rx.clone(),
            wake,
            persisted: Arc::new(AtomicU64::new(0)),
            failed: Arc::new(AtomicU64::new(0)),
            reported_failure: AtomicU64::new(0),
            running: Arc::new(AtomicBool::new(true)),
            next: 0,
        };
        writer
            .submit(vec![("last_folder", "folder".into())])
            .unwrap();
        for _ in 0..100 {
            writer.submit(vec![("view_mode", "list".into())]).unwrap();
        }
        let (_, snapshot, _) = rx.try_recv().unwrap();
        assert!(snapshot.contains(&("last_folder", "folder".into())));
        assert!(snapshot.contains(&("view_mode", "list".into())));
        assert!(rx.try_recv().is_err());
    }
}
