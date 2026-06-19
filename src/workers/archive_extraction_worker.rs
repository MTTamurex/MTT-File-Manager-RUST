//! Dedicated worker for native archive extraction.
//!
//! Runs on its own thread so that long-running CPU/IO-bound extraction does not
//! block the file-operation worker queue (delete, rename, paste, properties, etc.).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};

use crate::infrastructure::archive_extract::{
    self, ExtractionCancelFlag, SharedExtractionProgress,
};
use crate::infrastructure::diagnostic_logger::{diag_error, field_label};
use crate::workers::file_operation_worker::FileOperationResult;

/// Request sent from the file-operation worker to the archive extraction worker.
pub(crate) enum ArchiveExtractionRequest {
    Copy {
        paths: Vec<PathBuf>,
        dest_folder: PathBuf,
        copied_dests: Vec<PathBuf>,
    },
    MoveSingle {
        paths: Vec<PathBuf>,
        dest_folder: PathBuf,
        source_folder: PathBuf,
        moved_dest: Option<PathBuf>,
    },
    MoveBatch {
        paths: Vec<PathBuf>,
        dest_folder: PathBuf,
        source_folders: Vec<PathBuf>,
        moved_files: Vec<PathBuf>,
    },
}

/// Starts the archive extraction worker thread.
///
/// The worker processes one extraction request at a time. It shares the same
/// `SharedExtractionProgress` and `ExtractionCancelFlag` with the UI, so the
/// existing extraction toast and cancel button work without modification.
pub(crate) fn start_archive_extraction_worker(
    receiver: Receiver<ArchiveExtractionRequest>,
    result_sender: Sender<FileOperationResult>,
    extraction_progress: SharedExtractionProgress,
    extraction_cancel: ExtractionCancelFlag,
) {
    let spawn_result = crate::spawn_named("archive-extract-worker", move || {
        while let Ok(request) = receiver.recv() {
            // Reset cancel flag at the start of each extraction job only.
            extraction_cancel.store(false, Ordering::Relaxed);

            let progress = extraction_progress.clone();
            let cancel = extraction_cancel.clone();

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match request {
                ArchiveExtractionRequest::Copy {
                    paths,
                    dest_folder,
                    copied_dests,
                } => {
                    let success = archive_extract::extract_files_from_archive(
                        &paths,
                        &dest_folder,
                        &progress,
                        &cancel,
                    );
                    if success {
                        let _ = result_sender.send(FileOperationResult::CopyCompleted {
                            dest_folder,
                            copied_dests,
                        });
                    } else {
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
                    }
                }
                ArchiveExtractionRequest::MoveSingle {
                    paths,
                    dest_folder,
                    source_folder,
                    moved_dest,
                } => {
                    let success = archive_extract::extract_files_from_archive(
                        &paths,
                        &dest_folder,
                        &progress,
                        &cancel,
                    );
                    if success {
                        let source_path = paths
                            .first()
                            .cloned()
                            .unwrap_or_else(|| source_folder.clone());
                        let _ = result_sender.send(FileOperationResult::MoveCompleted {
                            source_folder,
                            dest_folder,
                            source_path,
                            moved_dest,
                        });
                    } else {
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
                    }
                }
                ArchiveExtractionRequest::MoveBatch {
                    paths,
                    dest_folder,
                    source_folders,
                    moved_files,
                } => {
                    let success = archive_extract::extract_files_from_archive(
                        &paths,
                        &dest_folder,
                        &progress,
                        &cancel,
                    );
                    if success {
                        if !source_folders.is_empty() {
                            let _ = result_sender.send(FileOperationResult::MoveBatchCompleted {
                                source_folders,
                                dest_folder,
                                moved_files,
                            });
                        }
                    } else {
                        let _ = result_sender.send(FileOperationResult::OperationFailed {
                            message: rust_i18n::t!("operations.error_cancelled").to_string(),
                        });
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
                log::error!("[ArchiveExtractWorker] worker thread panicked");
                diag_error(
                    "archive_extraction_worker",
                    "worker_panic",
                    &[field_label("payload_kind", panic_payload)],
                );
                let _ = result_sender.send(FileOperationResult::OperationFailed { message: msg });

                // Clear progress on panic so the toast doesn't get stuck.
                if let Ok(mut guard) = extraction_progress.lock() {
                    *guard = None;
                }
            }

            // Always signal completion so file_ops_in_progress is decremented.
            let _ = result_sender.send(FileOperationResult::FinishedNoRefresh);
        }
    });

    if let Err(error) = spawn_result {
        log::error!(
            "[ArchiveExtractWorker] failed to spawn worker thread: {}",
            error
        );
        diag_error("archive_extraction_worker", "spawn_failed", &[]);
    }
}
