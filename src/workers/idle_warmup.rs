use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::prefetch_worker::PrefetchMessage;
use crate::workers::thumbnail_worker::PriorityThumbnailQueue;

const IDLE_THRESHOLD: Duration = Duration::from_secs(5);
const WARMUP_INTERVAL: Duration = Duration::from_millis(200);

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

    pub fn is_idle(&self) -> bool {
        self.last_activity.elapsed() >= IDLE_THRESHOLD
    }

    pub fn on_activity(&mut self) {
        self.last_activity = Instant::now();
        self.is_warming = false;
    }
}

pub fn spawn_idle_warmup_worker(
    receiver: Receiver<IdleWarmupMessage>,
    thumbnail_queue: Arc<PriorityThumbnailQueue>,
    directory_cache: Arc<DirectoryCache>,
    current_generation: Arc<std::sync::atomic::AtomicUsize>,
    prefetch_sender: std::sync::mpsc::Sender<PrefetchMessage>,
) {
    std::thread::spawn(move || {
        io_priority::set_thread_priority(IOPriority::Background);

        let mut worker = IdleWarmupWorker::new();
        let mut last_warmup = Instant::now();

        loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(IdleWarmupMessage::UserActive) => {
                    worker.on_activity();
                }
                Ok(IdleWarmupMessage::CurrentDirectory(path)) => {
                    worker.current_directory = Some(path);
                }
                Ok(IdleWarmupMessage::VisibleItems(items)) => {
                    worker.pending_thumbnails = items;
                }
                Ok(IdleWarmupMessage::Shutdown) => {
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }

            if !worker.is_idle() {
                continue;
            }

            if last_warmup.elapsed() < WARMUP_INTERVAL {
                continue;
            }

            worker.is_warming = true;

            if let Some(path) = worker.pending_thumbnails.pop() {
                let gen = current_generation.load(Ordering::Relaxed);
                thumbnail_queue.push(path, gen, 256, IOPriority::Background);
                last_warmup = Instant::now();
                continue;
            }

            if let Some(ref current) = worker.current_directory {
                if let Some(entries) = directory_cache.get(current) {
                    let subdirs: Vec<PathBuf> = entries
                        .iter()
                        .filter(|e| e.is_dir)
                        .filter(|e| directory_cache.get(&e.path).is_none())
                        .take(3)
                        .map(|e| e.path.clone())
                        .collect();
                    if !subdirs.is_empty() {
                        let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
                        last_warmup = Instant::now();
                    }
                }
            }
        }
    });
}
