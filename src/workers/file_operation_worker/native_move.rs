//! Manual Move/MoveBatch only. Organizer and virtual transfers stay unchanged.
use super::{
    sanitize_operation_path, sanitize_operation_paths, FileOperationRequest, FileOperationResult,
};
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome, OutcomeKind};
use std::sync::{atomic::Ordering, mpsc::Sender, Arc};

pub(super) fn try_handle(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
    runtime: &Arc<CopyRuntime>,
) -> bool {
    if !runtime.enabled.load(Ordering::Relaxed) {
        return false;
    }
    let (paths, destination, token) = match request {
        FileOperationRequest::Move {
            path, dest_folder, ..
        } => (std::slice::from_ref(path), dest_folder, None),
        FileOperationRequest::MoveBatch {
            paths,
            dest_folder,
            clipboard_paste_token,
            ..
        } => (paths.as_slice(), dest_folder, *clipboard_paste_token),
        _ => return false,
    };
    if !super::shell_transfers::is_filesystem_transfer(paths, destination) {
        return false;
    }
    let job = runtime.start_move(paths.to_vec(), destination.clone());
    let _monitor =
        crate::infrastructure::copy_engine::diagnostics::Monitor::start(job.operation.clone());
    let result = sanitize_operation_paths(paths)
        .and_then(|paths| {
            let destination = sanitize_operation_path(destination)?;
            Ok((paths, destination))
        })
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Move path validation failed",
            )
        })
        .and_then(|(paths, destination)| {
            mtt_copy_engine::move_with_conflicts(&paths, &destination, &job.operation.progress)
        });
    let outcome = outcome(&result);
    let complete = result.as_ref().is_ok_and(|summary| {
        !summary.cancelled
            && summary.failures == 0
            && summary.items_skipped == 0
            && summary.sources_retained == 0
    });
    let pairs = result
        .as_ref()
        .map(|summary| summary.moved_pairs.clone())
        .unwrap_or_default();
    let mut source_folders: Vec<_> = paths
        .iter()
        .filter_map(|path| path.parent().map(std::path::Path::to_path_buf))
        .collect();
    source_folders.extend(
        pairs
            .iter()
            .filter_map(|(path, _)| path.parent().map(std::path::Path::to_path_buf)),
    );
    source_folders.sort();
    source_folders.dedup();
    // Send even for partial/cancelled jobs. Only identity-confirmed removals may
    // remap tags/caches. Publication alone is not a completed source movement.
    let _ = sender.send(FileOperationResult::MoveBatchCompleted {
        source_folders,
        dest_folder: destination.clone(),
        moved_files: pairs.iter().map(|(source, _)| source.clone()).collect(),
        known_moved_pairs: pairs,
        clipboard_paste_token: token.filter(|_| complete),
    });
    // Retained copies and partial roots still need destination view refresh.
    let _ = sender.send(FileOperationResult::CopyCompleted {
        dest_folder: destination.clone(),
        copied_dests: result
            .as_ref()
            .map(|summary| summary.destinations.clone())
            .unwrap_or_default(),
    });
    if !complete {
        if let Some(token) = token {
            let _ = sender.send(FileOperationResult::ClipboardMoveFailed {
                token,
                message: rust_i18n::t!("move_engine.incomplete").to_string(),
            });
        }
    }
    job.finish(outcome);
    true
}

fn outcome(result: &std::io::Result<mtt_copy_engine::Summary>) -> Outcome {
    let cancelled = match result {
        Ok(summary) => summary.cancelled,
        Err(error) => error.kind() == std::io::ErrorKind::Interrupted,
    };
    let failed = match result {
        Ok(summary) => {
            summary.failures > 0 || (summary.sources_retained > 0 && summary.items_skipped == 0)
        }
        Err(_) => true,
    };
    let kind = if cancelled {
        OutcomeKind::Cancelled
    } else if failed {
        OutcomeKind::Failed
    } else {
        OutcomeKind::Completed
    };
    let mut messages = Vec::new();
    if cancelled {
        messages.push(rust_i18n::t!("move_engine.cancelled").to_string());
    }
    match result {
        Ok(summary) => {
            if summary.sources_retained > 0 {
                messages.push(
                    rust_i18n::t!("move_engine.retained", count = summary.sources_retained)
                        .to_string(),
                );
            }
            if let Some(error) = &summary.first_error {
                messages.push(format!(
                    "{}: {error}",
                    rust_i18n::t!("copy_engine.failed", count = summary.failures)
                ));
            }
        }
        Err(error) if !cancelled => {
            messages.push(format!("{}: {error}", rust_i18n::t!("copy_engine.error")))
        }
        _ => {}
    }
    Outcome {
        kind,
        message: (!messages.is_empty()).then(|| messages.join("\n")),
    }
}

#[cfg(test)]
mod tests;
