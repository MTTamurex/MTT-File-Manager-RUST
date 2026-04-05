use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

pub struct FileOperationState {
    pub(crate) file_op_sender: Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    pub file_op_res_receiver: Receiver<crate::workers::file_operation_worker::FileOperationResult>,
    pub extraction_progress: crate::infrastructure::archive_extract::SharedExtractionProgress,
    pub extraction_cancel: crate::infrastructure::archive_extract::ExtractionCancelFlag,
    pub disk_cache_invalidation_sender:
        Sender<Vec<crate::app::init_workers::CacheInvalidationEntry>>,
    pub prefetch_sender: Sender<crate::workers::prefetch_worker::PrefetchMessage>,
    pub idle_warmup_sender: Sender<crate::workers::idle_warmup::IdleWarmupMessage>,
    pub file_ops_in_progress: usize,
    pub pending_deletions: Arc<dashmap::DashMap<PathBuf, ()>>,
    pub pending_iso_mount: Option<PathBuf>,
}
