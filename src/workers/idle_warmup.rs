use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::thumbnail::PriorityThumbnailQueue;

pub enum IdleWarmupMessage {
    UserActive,
    CurrentDirectory(PathBuf),
    VisibleItems(Vec<PathBuf>),
    Shutdown,
}

pub struct IdleWarmupWorker {
    last_activity: Instant,
    current_directory: Option<PathBuf>,
    pending_thumbnails: Vec<PathBuf>,
    is_warming: bool,
}

impl IdleWarmupWorker {
    pub fn new() -> Self {
        Self {
            last_activity: Instant::now(),
            current_directory: None,
            pending_thumbnails: Vec::new(),
            is_warming: false,
        }
    }

    pub fn on_activity(&mut self) {
        self.last_activity = Instant::now();
        self.is_warming = false;
    }
}

impl Default for IdleWarmupWorker {
    fn default() -> Self {
        Self::new()
    }
}

pub fn spawn_idle_warmup_worker(
    receiver: Receiver<IdleWarmupMessage>,
    _thumbnail_queue: Arc<PriorityThumbnailQueue>,
    _directory_cache: Arc<DirectoryCache>,
    _current_generation: Arc<std::sync::atomic::AtomicUsize>,
) {
    std::thread::spawn(move || {
        let _priority_guard = io_priority::ThreadPriorityGuard::new(IOPriority::Background);

        let mut worker = IdleWarmupWorker::new();

        loop {
            // BLOCKING: Wait for message instead of polling
            match receiver.recv() {
                Ok(IdleWarmupMessage::UserActive) => {
                    worker.on_activity();
                    continue; // Process next message immediately
                }
                Ok(IdleWarmupMessage::CurrentDirectory(path)) => {
                    worker.current_directory = Some(path);
                    continue;
                }
                Ok(IdleWarmupMessage::VisibleItems(items)) => {
                    worker.pending_thumbnails = items;
                    continue;
                }
                Ok(IdleWarmupMessage::Shutdown) => {
                    break;
                }
                Err(_) => {
                    break; // Channel closed
                }
            }
        }
    });
}
