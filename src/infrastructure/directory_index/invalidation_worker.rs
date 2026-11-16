//! Delete notifications must not wait for an index reader/writer on the UI.
use super::DirectoryIndex;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

type Batch = Vec<(PathBuf, bool)>;

#[derive(Default)]
pub(super) struct Invalidations {
    sender: Mutex<Option<mpsc::Sender<Batch>>>,
}

impl DirectoryIndex {
    /// Queue exact listings or recursive deleted-directory invalidations.
    /// The caller marks memory listings dirty before using this non-blocking path.
    pub(crate) fn invalidate_later(self: &Arc<Self>, batch: Batch) -> bool {
        if batch.is_empty() {
            return true;
        }
        let mut slot = self.invalidations.sender.lock();
        if slot.is_none() {
            let (sender, receiver) = mpsc::channel::<Batch>();
            let index = Arc::downgrade(self);
            if std::thread::Builder::new()
                .name("delete-index-cleanup".into())
                .spawn(move || {
                    while let Ok(batch) = receiver.recv() {
                        let mut paths: HashMap<PathBuf, bool> = HashMap::new();
                        for (path, recursive) in batch
                            .into_iter()
                            .chain(receiver.try_iter().take(31).flatten())
                        {
                            *paths.entry(path).or_default() |= recursive;
                        }
                        let Some(index) = index.upgrade() else { break };
                        for (path, recursive) in paths {
                            let result = if recursive {
                                index.invalidate_recursive(&path)
                            } else {
                                index.invalidate(&path)
                            };
                            if result.is_err() {
                                log::warn!("[DELETE] Persistent listing invalidation failed");
                            }
                        }
                    }
                })
                .is_err()
            {
                return false;
            }
            *slot = Some(sender);
        }
        let result = slot.as_ref().unwrap().send(batch);
        if let Err(error) = result {
            *slot = None;
            drop(slot);
            return self.invalidate_later(error.0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn enqueue_does_not_wait_for_the_sqlite_connection() {
        let index = Arc::new(DirectoryIndex {
            conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
            invalidations: Default::default(),
        });
        let connection = index.conn.lock();
        connection
            .execute_batch(
                "CREATE TABLE file_index(dir_path TEXT);
             CREATE TABLE directory_index(dir_path TEXT);",
            )
            .unwrap();
        let other = index.clone();
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let queued = other.invalidate_later(vec![(PathBuf::from(r"C:\Data"), false)]);
            sender.send(queued).unwrap();
        });
        let queued = receiver.recv_timeout(Duration::from_secs(2));
        drop(connection);
        thread.join().unwrap();
        assert!(queued.unwrap());
    }
}
