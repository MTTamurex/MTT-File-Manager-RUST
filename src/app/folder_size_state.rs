use crate::ui::cache::FxHashSet;
use lru::LruCache;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum FolderSizeMessage {
    Progress {
        folder_path: PathBuf,
        total_size: u64,
    },
    Complete {
        folder_path: PathBuf,
        total_size: u64,
    },
    Cancelled {
        folder_path: PathBuf,
    },
}

pub struct FolderSizeState {
    pub req_sender: Sender<PathBuf>,
    pub res_receiver: Receiver<FolderSizeMessage>,
    pub cancel: Arc<AtomicBool>,
    pub cache: LruCache<PathBuf, u64>,
    pub loading: FxHashSet<PathBuf>,
}
