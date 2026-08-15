use crate::infrastructure::directory_cache::DirectoryCache;
use crate::workers::idle_warmup::IdleWarmupMessage;
use crate::workers::prefetch_worker::PrefetchMessage;
use crate::workers::thumbnail::PriorityThumbnailQueue;
use eframe::egui;
use std::sync::{mpsc, Arc};

pub(in crate::app) struct PrefetchWorkerHandles {
    pub(in crate::app) prefetch_sender: mpsc::Sender<PrefetchMessage>,
    pub(in crate::app) idle_warmup_sender: mpsc::Sender<IdleWarmupMessage>,
}

pub(in crate::app) fn spawn_prefetching_workers(
    directory_cache: Arc<DirectoryCache>,
    thumbnail_queue: Arc<PriorityThumbnailQueue>,
    shared_gen: Arc<std::sync::atomic::AtomicUsize>,
) -> PrefetchWorkerHandles {
    let (prefetch_tx, prefetch_rx) = mpsc::channel();
    crate::workers::prefetch_worker::spawn_prefetch_worker(prefetch_rx, directory_cache.clone());

    let (idle_warmup_tx, idle_warmup_rx) = mpsc::channel();
    crate::workers::idle_warmup::spawn_idle_warmup_worker(
        idle_warmup_rx,
        thumbnail_queue,
        directory_cache,
        shared_gen,
    );

    PrefetchWorkerHandles {
        prefetch_sender: prefetch_tx,
        idle_warmup_sender: idle_warmup_tx,
    }
}

pub(in crate::app) fn spawn_file_operation_worker() -> (
    crossbeam_channel::Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    mpsc::Receiver<crate::workers::file_operation_worker::FileOperationResult>,
    crate::infrastructure::archive_extract::SharedExtractionProgress,
    crate::infrastructure::archive_extract::ExtractionCancelFlag,
    mpsc::Sender<crate::workers::archive_compression_worker::ArchiveCompressionRequest>,
    crate::infrastructure::archive_create::SharedCompressionProgress,
    crate::infrastructure::archive_create::CompressionCancelFlag,
) {
    // Multi-consumer request channel: the file-operation worker pool pulls
    // requests concurrently so long transfers don't serialize quick ops.
    let (file_op_tx, file_op_rx) = crossbeam_channel::unbounded();
    let (file_op_res_tx, file_op_res_rx) = mpsc::channel();
    let extraction_progress = crate::infrastructure::archive_extract::new_shared_progress();
    let extraction_cancel = crate::infrastructure::archive_extract::new_cancel_flag();

    // Create archive extraction channel and start the dedicated worker.
    let (archive_extract_tx, archive_extract_rx) = mpsc::channel();
    crate::workers::archive_extraction_worker::start_archive_extraction_worker(
        archive_extract_rx,
        file_op_res_tx.clone(),
        extraction_progress.clone(),
        extraction_cancel.clone(),
    );

    // Create archive compression channel and start the dedicated worker.
    let compression_progress = crate::infrastructure::archive_create::new_shared_progress();
    let compression_cancel = crate::infrastructure::archive_create::new_cancel_flag();
    let (archive_compress_tx, archive_compress_rx) = mpsc::channel();
    crate::workers::archive_compression_worker::start_archive_compression_worker(
        archive_compress_rx,
        file_op_res_tx.clone(),
        compression_progress.clone(),
        compression_cancel.clone(),
    );

    crate::workers::file_operation_worker::start_file_operation_worker(
        Arc::new(file_op_rx),
        file_op_res_tx,
        archive_extract_tx,
    );
    (
        file_op_tx,
        file_op_res_rx,
        extraction_progress,
        extraction_cancel,
        archive_compress_tx,
        compression_progress,
        compression_cancel,
    )
}

pub(in crate::app) fn spawn_global_search_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<crate::workers::global_search_worker::GlobalSearchRequest>,
    mpsc::Receiver<crate::workers::global_search_worker::GlobalSearchResponse>,
) {
    let (global_search_tx, global_search_rx_thread) = mpsc::channel();
    let (global_search_res_tx, global_search_res_rx) = mpsc::channel();
    crate::workers::global_search_worker::start_global_search_worker(
        global_search_rx_thread,
        global_search_res_tx,
        ctx.clone(),
    );
    (global_search_tx, global_search_res_rx)
}
