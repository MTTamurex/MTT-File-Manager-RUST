//! Optional engine dispatch; the default Shell and virtual namespace paths remain intact.
use super::{sanitize_operation_paths, FileOperationRequest, FileOperationResult};
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome, OutcomeKind};
use crate::infrastructure::delete_engine::{confirmation, permanent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{atomic::Ordering, mpsc::Sender, Arc};
mod recycling;
mod refresh;

pub(super) fn try_handle(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
    runtime: &Arc<CopyRuntime>,
) -> bool {
    handle_with_confirmation(request, sender, runtime, confirmation::ask)
}

fn handle_with_confirmation(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
    runtime: &Arc<CopyRuntime>,
    confirm: impl Fn(
        &crate::infrastructure::copy_engine::CopyOperation,
        confirmation::Kind,
        &[PathBuf],
    ) -> bool,
) -> bool {
    if !runtime.enabled.load(Ordering::Relaxed) {
        return false;
    }
    let (paths, permanently, hwnd) = match request {
        FileOperationRequest::Delete { paths, hwnd } => (paths, false, hwnd.0),
        FileOperationRequest::DeletePermanently {
            physical_paths,
            hwnd,
        } => (physical_paths, true, hwnd.0),
        FileOperationRequest::SearchDelete {
            paths,
            permanently,
            hwnd,
        } => (paths, *permanently, hwnd.0),
        _ => return false,
    };
    // Recycle-bin namespace and other virtual objects keep their original path.
    if paths.is_empty()
        || paths.iter().any(|path| {
            !path.is_absolute()
                || path.components().any(|component| {
                    let name = component.as_os_str().to_string_lossy();
                    name.eq_ignore_ascii_case("$Recycle.Bin")
                        || name.eq_ignore_ascii_case("Recycler")
                        || name.eq_ignore_ascii_case("Recycled")
                })
        })
    {
        return false;
    }
    let job = runtime.start_delete(paths.clone(), permanently);
    let operation = &job.operation;
    let progress = operation.deletion.as_ref().unwrap();
    let validated = paths
        .iter()
        .try_for_each(|path| {
            if path.parent().is_none()
                || crate::domain::file_entry::is_path_inside_existing_archive_file(path)
            {
                return Err("Invalid deletion root".to_string());
            }
            // Never canonicalize a selected link into its target before deletion.
            crate::infrastructure::security::validate_no_reparse_points(path)
                .map_err(|_| "Reparse path refused".to_string())
        })
        .and_then(|()| sanitize_operation_paths(paths).map(|_| paths.clone()));
    let paths = match validated {
        Ok(paths) => unique_roots(paths),
        Err(_) => {
            job.finish(Outcome {
                kind: OutcomeKind::Failed,
                message: Some(rust_i18n::t!("operations.selection_operation_blocked").to_string()),
            });
            return true;
        }
    };
    {
        let kind = if permanently {
            confirmation::Kind::Permanent
        } else {
            confirmation::Kind::Recycle
        };
        if !confirm(operation, kind, &paths) || operation.progress.is_cancelled() {
            job.finish(Outcome {
                kind: OutcomeKind::Cancelled,
                message: Some(rust_i18n::t!("delete_engine.cancelled").to_string()),
            });
            return true;
        }
    }
    let existing: Vec<_> = paths
        .iter()
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .cloned()
        .collect();
    let removed = Arc::new(std::sync::Mutex::new(refresh::RemovedItems::new(
        sender.clone(),
    )));
    let observer: crate::infrastructure::delete_engine::RemovalObserver = {
        let removed = removed.clone();
        Arc::new(move |path| {
            removed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record(path)
        })
    };
    let completed = if permanently {
        for path in &paths {
            if operation.progress.is_cancelled() {
                break;
            }
            if let Err(error) = permanent::delete_with_observer(path, operation, &observer) {
                if error.kind() != std::io::ErrorKind::Interrupted {
                    progress.failed();
                }
            }
        }
        !operation.progress.is_cancelled()
    } else {
        recycling::run(&paths, hwnd, operation, observer, &confirm)
    };
    removed.lock().unwrap_or_else(|e| e.into_inner()).flush();
    progress.set_current(None);
    let deleted_paths: Vec<_> = existing.iter().filter(|path| matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)).cloned().collect();
    if completed
        && !operation.progress.is_cancelled()
        && deleted_paths.len() != existing.len()
        && progress.failures.load(Ordering::Relaxed) == 0
    {
        progress.failed();
    }
    // Refresh all selected parents, even when only some children were removed.
    let parents: HashSet<_> = paths
        .iter()
        .filter_map(|path| path.parent().map(|p| p.to_path_buf()))
        .collect();
    let _ = sender.send(FileOperationResult::DeleteCompleted {
        parent_folders: parents.into_iter().collect(),
        // Direct removals have already been reported in bounded child batches.
        deleted_paths: if permanently {
            Vec::new()
        } else {
            deleted_paths
        },
    });
    if !permanently {
        let _ = sender.send(FileOperationResult::RecycleBinChanged);
    }
    let failures = progress.failures.load(Ordering::Relaxed);
    let kind = if failures > 0 {
        OutcomeKind::Failed
    } else if !completed || operation.progress.is_cancelled() {
        OutcomeKind::Cancelled
    } else {
        OutcomeKind::Completed
    };
    let message = match kind {
        OutcomeKind::Completed => None,
        OutcomeKind::Cancelled => Some(rust_i18n::t!("delete_engine.cancelled").to_string()),
        OutcomeKind::Failed => {
            Some(rust_i18n::t!("delete_engine.failed", count = failures).to_string())
        }
    };
    job.finish(Outcome { kind, message });
    true
}

fn unique_roots(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut sorted = paths;
    sorted.sort_by_key(|path| path.components().count());
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut keys = HashSet::new();
    for path in sorted {
        if !path
            .ancestors()
            .any(|ancestor| keys.contains(&ancestor.as_os_str().to_string_lossy().to_lowercase()))
        {
            keys.insert(path.as_os_str().to_string_lossy().to_lowercase());
            roots.push(path);
        }
    }
    roots
}

#[cfg(test)]
mod tests;
