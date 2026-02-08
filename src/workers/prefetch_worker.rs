use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use crate::domain::file_entry::{FileEntry, SyncStatus};
use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::ntfs_reader;

const MAX_PREFETCH_DIRS: usize = 5;

pub enum PrefetchMessage {
    Prefetch(Vec<PathBuf>),
    Shutdown,
}

pub fn spawn_prefetch_worker(
    receiver: Receiver<PrefetchMessage>,
    directory_cache: Arc<DirectoryCache>,
) {
    std::thread::spawn(move || {
        io_priority::set_thread_priority(IOPriority::Background);

        while let Ok(msg) = receiver.recv() {
            match msg {
                PrefetchMessage::Prefetch(paths) => {
                    for path in paths.into_iter().take(MAX_PREFETCH_DIRS) {
                        if directory_cache.get(&path).is_some() {
                            continue;
                        }

                        // Skip prefetch caching for SSDs - raw disk speed is sufficient
                        if io_priority::is_ssd(&path) {
                            continue;
                        }

                        if let Some(entries) = ntfs_reader::read_directory_fast(&path) {
                            let file_entries: Vec<FileEntry> = entries
                                .into_iter()
                                .filter(|e| {
                                    let is_hidden = (e.attributes & 0x02) != 0;
                                    let is_system = (e.attributes & 0x04) != 0;
                                    !is_hidden && !is_system && !e.name.starts_with('.')
                                })
                                .map(|e| FileEntry {
                                    path: path.join(&e.name),
                                    name: e.name,
                                    is_dir: e.is_dir,
                                    size: if e.is_dir { 0 } else { e.size },
                                    modified: e.modified,
                                    folder_cover: None,
                                    drive_info: None,
                                    sync_status: SyncStatus::None,
                                    deletion_date: None,
                                    recycle_original_path: None,
                                })
                                .collect();

                            directory_cache.put(path, file_entries);
                        }
                    }
                }
                PrefetchMessage::Shutdown => break,
            }
        }

        io_priority::reset_thread_priority();
    });
}
