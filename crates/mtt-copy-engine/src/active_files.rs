//! In-memory tracking bounded by the number of active copy workers, not the queue.
use crate::Progress;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub(crate) struct ActiveFiles {
    next: AtomicU64,
    files: Mutex<BTreeMap<u64, PathBuf>>,
}

impl Progress {
    /// A short snapshot; never keep the registry locked while rendering or doing I/O.
    pub fn active_files(&self) -> Vec<PathBuf> {
        self.active
            .files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub(crate) fn track_file(&self, path: &Path) -> ActiveFile<'_> {
        let id = self.active.next.fetch_add(1, Ordering::Relaxed);
        self.active
            .files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, path.to_path_buf());
        ActiveFile { progress: self, id }
    }
}

pub(crate) struct ActiveFile<'a> {
    progress: &'a Progress,
    id: u64,
}

impl Drop for ActiveFile<'_> {
    fn drop(&mut self) {
        self.progress
            .active
            .files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_files_are_ordered_and_removed_independently_even_for_equal_paths() {
        let progress = Progress::default();
        let first = progress.track_file(Path::new("folder/file.txt"));
        let second = progress.track_file(Path::new("other/file.txt"));
        let duplicate = progress.track_file(Path::new("folder/file.txt"));
        assert_eq!(
            progress.active_files(),
            vec![
                PathBuf::from("folder/file.txt"),
                PathBuf::from("other/file.txt"),
                PathBuf::from("folder/file.txt")
            ]
        );
        drop(first);
        assert_eq!(progress.active_files().len(), 2);
        drop(duplicate);
        assert_eq!(
            progress.active_files(),
            vec![PathBuf::from("other/file.txt")]
        );
        drop(second);
        assert!(progress.active_files().is_empty());
    }

    #[test]
    fn failed_or_panicking_transfers_do_not_leave_stale_names() {
        let progress = Progress::default();
        let failed = || -> Result<(), ()> {
            let _file = progress.track_file(Path::new("file.txt"));
            Err(())
        };
        assert!(failed().is_err());
        assert!(progress.active_files().is_empty());
        let _ = std::panic::catch_unwind(|| {
            let _file = progress.track_file(Path::new("file.txt"));
            panic!("Test transfer failure");
        });
        assert!(progress.active_files().is_empty());
    }

    #[test]
    fn pausing_keeps_the_active_name_and_cancellation_removes_it() {
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, Instant};
        let progress = Arc::new(Progress::default());
        progress.pause();
        let (ready, receiver) = mpsc::channel();
        std::thread::scope(|scope| {
            let worker = progress.clone();
            scope.spawn(move || {
                let _participant = worker.participant();
                let _file = worker.track_file(Path::new("file.txt"));
                ready.send(()).unwrap();
                assert!(worker.checkpoint().is_err());
            });
            receiver.recv_timeout(Duration::from_secs(3)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while progress.pause_state() != crate::PauseState::Paused && Instant::now() < deadline {
                std::thread::yield_now();
            }
            let paused = progress.pause_state() == crate::PauseState::Paused;
            let names = progress.active_files();
            progress.cancel();
            assert!(paused);
            assert_eq!(names, vec![PathBuf::from("file.txt")]);
        });
        assert!(progress.active_files().is_empty());
    }
}
