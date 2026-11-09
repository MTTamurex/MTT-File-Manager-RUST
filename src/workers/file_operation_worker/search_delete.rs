//! Search deletions use one explicit warning for the entire mixed-origin selection.
use super::{sanitize_operation_paths, FileOperationResult, SendHwnd};
use crate::infrastructure::windows::shell_operations;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub(super) fn handle(
    paths: Vec<PathBuf>,
    permanently: bool,
    hwnd: SendHwnd,
    sender: &Sender<FileOperationResult>,
) {
    let Ok(paths) = sanitize_operation_paths(&paths) else {
        let _ = sender.send(FileOperationResult::OperationFailed {
            message: rust_i18n::t!("operations.selection_operation_blocked").to_string(),
        });
        return;
    };
    if paths.is_empty() {
        return;
    }
    let outcome = match shell_operations::delete_search_selection(&paths, permanently, hwnd.0) {
        Ok(Some(outcome)) => outcome,
        Ok(None) => return, // Cancelling the consolidated confirmation is not an error.
        Err(error) => {
            log::error!(
                "[SearchDelete] Could not display confirmation: {}",
                error.code()
            );
            let _ = sender.send(FileOperationResult::OperationFailed {
                message: rust_i18n::t!("operations.error_delete_confirmation").to_string(),
            });
            return;
        }
    };
    if !outcome.deleted_paths.is_empty() {
        let parents: HashSet<_> = outcome
            .deleted_paths
            .iter()
            .filter_map(|path| path.parent().map(|parent| parent.to_path_buf()))
            .collect();
        let _ = sender.send(FileOperationResult::DeleteCompleted {
            parent_folders: parents.into_iter().collect(),
            deleted_paths: outcome.deleted_paths,
        });
        let _ = sender.send(FileOperationResult::RecycleBinChanged);
    }
    if !outcome.completed {
        let _ = sender.send(FileOperationResult::OperationFailed {
            message: rust_i18n::t!("operations.error_cancelled").to_string(),
        });
    }
}
