use crate::app::state::ItemsRebuildResult;
use crate::domain::file_entry::FileEntry;
use crate::domain::thumbnail::ThumbnailData;
use crate::ui::cache::FxHashSet;
use crate::workers::folder_preview_worker::FolderPreviewData;
use crate::workers::thumbnail::PriorityThumbnailQueue;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

/// Worker management state
pub struct WorkerState {
    /// Optimized thumbnail system
    pub thumbnail_queue: Arc<PriorityThumbnailQueue>,
    pub image_receiver: std::sync::mpsc::Receiver<ThumbnailData>,
    pub pending_thumbnails: VecDeque<ThumbnailData>, // PERFORMANCE: Buffer for throttled uploads

    /// Async file loading system
    pub file_entry_receiver: std::sync::mpsc::Receiver<(usize, Vec<FileEntry>)>,
    pub file_entry_sender: std::sync::mpsc::Sender<(usize, Vec<FileEntry>)>,
    pub is_loading_folder: bool,

    /// Async rebuild system (filter/sort)
    pub items_rebuild_sender: std::sync::mpsc::Sender<ItemsRebuildResult>,
    pub items_rebuild_receiver: std::sync::mpsc::Receiver<ItemsRebuildResult>,
    pub items_rebuild_request_id: usize,

    /// Folder cover worker
    pub cover_worker_sender: std::sync::mpsc::Sender<PathBuf>,
    pub cover_worker_receiver: std::sync::mpsc::Receiver<PathBuf>,
    pub scanned_folders: FxHashSet<PathBuf>,

    /// Folder preview worker (Native Windows Shell sandwich effect)
    /// UI sends PathBuf to the worker, worker sends FolderPreviewData back
    pub folder_preview_sender: std::sync::mpsc::Sender<PathBuf>,
    pub folder_preview_receiver: std::sync::mpsc::Receiver<FolderPreviewData>,

    /// Loaded items
    pub items: Arc<Vec<FileEntry>>,
}

impl WorkerState {
    pub fn new() -> Self {
        let (file_entry_sender, file_entry_receiver) = std::sync::mpsc::channel();
        let (items_rebuild_sender, items_rebuild_receiver) = std::sync::mpsc::channel();
        let (cover_worker_sender, cover_worker_receiver) = std::sync::mpsc::channel();
        let (_thumbnail_sender, thumbnail_receiver) = std::sync::mpsc::channel();

        // Create channels for folder preview worker
        let (folder_preview_sender, _folder_preview_to_worker) =
            std::sync::mpsc::channel::<PathBuf>();
        let (_folder_preview_from_worker, folder_preview_receiver) =
            std::sync::mpsc::channel::<FolderPreviewData>();

        Self {
            thumbnail_queue: Arc::new(PriorityThumbnailQueue::new()),
            image_receiver: thumbnail_receiver,
            pending_thumbnails: VecDeque::new(),
            file_entry_receiver,
            file_entry_sender,
            is_loading_folder: false,
            items_rebuild_sender,
            items_rebuild_receiver,
            items_rebuild_request_id: 0,
            cover_worker_sender,
            cover_worker_receiver,
            scanned_folders: FxHashSet::default(),
            folder_preview_sender,
            folder_preview_receiver,
            items: Arc::new(Vec::new()),
        }
    }

    /// Clear worker state
    pub fn clear(&mut self) {
        self.pending_thumbnails.clear();
        self.scanned_folders.clear();
        self.is_loading_folder = false;
        self.items_rebuild_request_id = 0;
    }

    /// Increment rebuild request ID
    pub fn increment_rebuild_request_id(&mut self) -> usize {
        self.items_rebuild_request_id += 1;
        self.items_rebuild_request_id
    }

    /// Create channels for folder preview worker following init.rs pattern
    pub fn create_folder_preview_channels() -> (
        std::sync::mpsc::Sender<PathBuf>,
        std::sync::mpsc::Receiver<FolderPreviewData>,
    ) {
        let (tx, _rx) = std::sync::mpsc::channel::<PathBuf>();
        let (_tx, rx) = std::sync::mpsc::channel::<FolderPreviewData>();
        (tx, rx)
    }
}

impl Default for WorkerState {
    fn default() -> Self {
        Self::new()
    }
}
