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
        prefetch_tx.clone(),
    );

    PrefetchWorkerHandles {
        prefetch_sender: prefetch_tx,
        idle_warmup_sender: idle_warmup_tx,
    }
}

pub(in crate::app) fn spawn_file_operation_worker() -> (
    mpsc::Sender<crate::workers::file_operation_worker::FileOperationRequest>,
    mpsc::Receiver<crate::workers::file_operation_worker::FileOperationResult>,
) {
    let (file_op_tx, file_op_rx) = mpsc::channel();
    let (file_op_res_tx, file_op_res_rx) = mpsc::channel();
    crate::workers::file_operation_worker::start_file_operation_worker(file_op_rx, file_op_res_tx);
    (file_op_tx, file_op_res_rx)
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
