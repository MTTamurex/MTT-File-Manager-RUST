//! Dedicated worker for native archive compression.
//!
//! Runs on its own thread so that long-running CPU/IO-bound compression does
//! not block the file-operation worker queue (delete, rename, paste, etc.).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};

use crate::infrastructure::archive_create::{
    self, CompressionCancelFlag, CompressionFormat, SharedCompressionProgress,
};
use crate::infrastructure::diagnostic_logger::{diag_error, field_label};
use crate::workers::file_operation_worker::FileOperationResult;

/// Request sent from the UI thread to the archive compression worker.
pub(crate) enum ArchiveCompressionRequest {
    Compress {
        sources: Vec<PathBuf>,
        dest: PathBuf,
        format: CompressionFormat,
    },
}

/// Starts the archive compression worker thread.
///
/// The worker processes one compression request at a time. It shares the same
/// `SharedCompressionProgress` and `CompressionCancelFlag` with the UI, so the
/// compression toast and cancel button work without extra plumbing.
pub(crate) fn start_archive_compression_worker(
    receiver: Receiver<ArchiveCompressionRequest>,
    result_sender: Sender<FileOperationResult>,
    compression_progress: SharedCompressionProgress,
    compression_cancel: CompressionCancelFlag,
) {
    let spawn_result = crate::spawn_named("archive-compress-worker", move || {
        while let Ok(request) = receiver.recv() {
            // Reset cancel flag at the start of each compression job only.
            compression_cancel.store(false, Ordering::Relaxed);

            let progress = compression_progress.clone();
            let cancel = compression_cancel.clone();

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match request {
                    ArchiveCompressionRequest::Compress {
                        sources,
                        dest,
                        format,
                    } => {
                        let dest_folder = dest
                            .parent()
                            .map(std::path::Path::to_path_buf)
                            .unwrap_or_else(|| dest.clone());
                        match archive_create::create_archive(
                            &sources, &dest, format, &progress, &cancel,
                        ) {
                            Ok(()) => {
                                let _ = result_sender.send(
                                    FileOperationResult::CompressionCompleted {
                                        dest_folder,
                                        archive_path: dest,
                                    },
                                );
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                                log::info!("[ArchiveCompressWorker] compression cancelled");
                                let _ = result_sender.send(FileOperationResult::OperationFailed {
                                    message: rust_i18n::t!("operations.compress_cancelled")
                                        .to_string(),
                                });
                            }
                            Err(e) => {
                                log::warn!("[ArchiveCompressWorker] compression failed: {}", e);
                                let _ = result_sender.send(FileOperationResult::OperationFailed {
                                    message: rust_i18n::t!(
                                        "operations.compress_failed",
                                        error = e.to_string()
                                    )
                                    .to_string(),
                                });
                            }
                        }
                    }
                }
            }));

            if let Err(e) = result {
                let (msg, panic_payload) = if let Some(s) = e.downcast_ref::<&str>() {
                    (s.to_string(), "str")
                } else if let Some(s) = e.downcast_ref::<String>() {
                    (s.clone(), "string")
                } else {
                    ("unknown".to_string(), "unknown")
                };
                log::error!("[ArchiveCompressWorker] worker thread panicked");
                diag_error(
                    "archive_compression_worker",
                    "worker_panic",
                    &[field_label("payload_kind", panic_payload)],
                );
                let _ = result_sender.send(FileOperationResult::OperationFailed { message: msg });

                // Clear progress on panic so the toast doesn't get stuck.
                if let Ok(mut guard) = compression_progress.lock() {
                    *guard = None;
                }
            }

            // Always signal completion so file_ops_in_progress is decremented.
            let _ = result_sender.send(FileOperationResult::FinishedNoRefresh);
        }
    });

    if let Err(error) = spawn_result {
        log::error!(
            "[ArchiveCompressWorker] failed to spawn worker thread: {}",
            error
        );
        diag_error("archive_compression_worker", "spawn_failed", &[]);
    }
}
