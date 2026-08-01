//! H-6: Graceful background-worker shutdown.
//!
//! All long-lived worker threads loop with `while let Ok(..) = rx.recv()`.
//! Dropping their `Sender` makes `recv()` return `Err(RecvError)`, which exits
//! the loop and lets the thread run its Drop impls (CoUninitialize, handles, …).

use crate::app::state::ImageViewerApp;
use std::sync::atomic::Ordering;

impl ImageViewerApp {
    /// Drop every persistent worker Sender so threads can exit cleanly.
    /// Called by `handle_exit` before the process terminates.
    pub fn shutdown_background_workers(&mut self) {
        // Invalidate queued context-menu work before disconnecting the channels.
        // Workers will skip stale queued requests instead of starting new COM calls.
        self.latest_shell_menu_request_id.store(
            self.shell_menu_request_id.wrapping_add(1),
            Ordering::Release,
        );
        self.pending_shell_menu_invocation_id
            .store(0, Ordering::Release);
        self.pending_open_with_invocation_id
            .store(0, Ordering::Release);
        let _ = self
            .shell_menu_control_tx
            .try_send(crate::infrastructure::shell_menu_worker::ShellMenuRequest::Cancel);
        let _ = self
            .open_with_control_tx
            .try_send(crate::infrastructure::open_with_worker::OpenWithRequest::Cancel);

        // Helper: replace a field with a disconnected channel of the same type so the
        // original Sender is dropped immediately, signalling the worker to exit.
        macro_rules! disconnect {
            ($field:expr, $T:ty) => {{
                let (d, _rx) = std::sync::mpsc::channel::<$T>();
                let _ = std::mem::replace(&mut $field, d);
            }};
        }

        macro_rules! disconnect_sync {
            ($field:expr, $T:ty) => {{
                let (d, _rx) = std::sync::mpsc::sync_channel::<$T>(1);
                let _ = std::mem::replace(&mut $field, d);
            }};
        }

        // Shell-menu STA thread
        disconnect_sync!(
            self.shell_menu_req_tx,
            crate::infrastructure::shell_menu_worker::ShellMenuRequest
        );
        disconnect_sync!(
            self.shell_menu_control_tx,
            crate::infrastructure::shell_menu_worker::ShellMenuRequest
        );
        disconnect_sync!(
            self.open_with_req_tx,
            crate::infrastructure::open_with_worker::OpenWithRequest
        );
        disconnect_sync!(
            self.open_with_control_tx,
            crate::infrastructure::open_with_worker::OpenWithRequest
        );

        // Disk-cache invalidation worker
        disconnect!(
            self.file_operation_state.disk_cache_invalidation_sender,
            Vec<crate::app::init_workers::CacheInvalidationEntry>
        );

        // Cover-image worker
        disconnect!(self.cover_worker_sender, std::path::PathBuf);

        // Folder-preview worker (crossbeam channel)
        {
            let (d, _rx) = crossbeam_channel::bounded::<
                crate::workers::folder_preview_worker::FolderPreviewRequest,
            >(1);
            let _ = std::mem::replace(&mut self.folder_preview_sender, d);
        }

        // Async icon worker
        disconnect!(self.icon_req_sender, (std::path::PathBuf, usize));

        // Async metadata worker
        disconnect!(self.metadata_req_sender, (std::path::PathBuf, u64));

        // Async live-file-size worker
        disconnect!(
            self.live_file_size_req_sender,
            crate::app::live_file_size::LiveFileSizeRequest
        );

        // Folder-size worker can stay busy in a long recursive scan, so cancel it first.
        self.folder_size_state
            .latest_request_id
            .store(0, Ordering::Release);
        self.folder_size_state.cancel.store(true, Ordering::Release);
        disconnect!(
            self.folder_size_state.req_sender,
            crate::app::folder_size_state::FolderSizeRequest
        );

        // Background prefetch pipeline.
        let _ = self
            .file_operation_state
            .prefetch_sender
            .send(crate::workers::prefetch_worker::PrefetchMessage::Shutdown);
        disconnect!(
            self.file_operation_state.prefetch_sender,
            crate::workers::prefetch_worker::PrefetchMessage
        );

        let _ = self
            .file_operation_state
            .idle_warmup_sender
            .send(crate::workers::idle_warmup::IdleWarmupMessage::Shutdown);
        disconnect!(
            self.file_operation_state.idle_warmup_sender,
            crate::workers::idle_warmup::IdleWarmupMessage
        );

        // Global search worker
        disconnect!(
            self.global_search.sender,
            crate::workers::global_search_worker::GlobalSearchRequest
        );

        // File-operation worker (type is pub(crate), accessible here)
        disconnect!(
            self.file_operation_state.file_op_sender,
            crate::workers::file_operation_worker::FileOperationRequest
        );

        // Consistency-probe worker (type is pub(super) → accessible within crate::app)
        disconnect!(
            self.consistency_probe_tx,
            super::super::init_workers::consistency_probe_worker::ConsistencyProbeRequest
        );
    }
}
