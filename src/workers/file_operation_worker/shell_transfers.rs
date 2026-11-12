//! Intercept only manual filesystem transfers. Archives, virtual namespaces and
//! organizer jobs retain their existing engines and completion semantics.

use super::{
    sanitize_operation_path, sanitize_operation_paths, FileOperationRequest, FileOperationResult,
};
use crate::domain::file_entry::is_path_inside_existing_archive_file;
use crate::infrastructure::security::classify_shell_namespace_path;
use crate::infrastructure::windows::shell_transfer::{self, DispatchOutcome, TransferKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

pub(super) fn is_filesystem_transfer(paths: &[PathBuf], destination: &Path) -> bool {
    !paths.is_empty()
        && std::iter::once(destination)
            .chain(paths.iter().map(PathBuf::as_path))
            .all(|path| {
                path.is_absolute()
                    && classify_shell_namespace_path(path).is_none()
                    && !is_path_inside_existing_archive_file(path)
            })
}

pub(super) fn try_delegate(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
) -> bool {
    let (paths, destination, owner, kind, clipboard_paste_token) = match request {
        FileOperationRequest::Copy {
            path,
            dest_folder,
            hwnd,
        } => (
            std::slice::from_ref(path),
            dest_folder,
            hwnd,
            TransferKind::Copy,
            None,
        ),
        FileOperationRequest::Move {
            path,
            dest_folder,
            hwnd,
        } => (
            std::slice::from_ref(path),
            dest_folder,
            hwnd,
            TransferKind::Move,
            None,
        ),
        FileOperationRequest::CopyBatch {
            paths,
            dest_folder,
            hwnd,
        } => (
            paths.as_slice(),
            dest_folder,
            hwnd,
            TransferKind::Copy,
            None,
        ),
        FileOperationRequest::MoveBatch {
            paths,
            dest_folder,
            hwnd,
            clipboard_paste_token,
        } => (
            paths.as_slice(),
            dest_folder,
            hwnd,
            TransferKind::Move,
            *clipboard_paste_token,
        ),
        _ => return false,
    };
    if !is_filesystem_transfer(paths, destination)
        || !shell_transfer::teracopy_copy_handler_enabled()
    {
        return false;
    }
    let (Ok(paths), Ok(destination)) = (
        sanitize_operation_paths(paths),
        sanitize_operation_path(destination),
    ) else {
        // Let the existing handler report validation failures as before.
        return false;
    };
    let outcome = shell_transfer::dispatch(&paths, &destination, owner.0, kind);
    log::debug!(
        "[ShellTransfer] Manual {kind:?} handoff: {outcome:?}; items={}",
        paths.len()
    );
    let Some(result) = handoff_result(&paths, destination, kind, clipboard_paste_token, outcome)
    else {
        return false;
    };
    let _ = sender.send(result);
    true
}

fn handoff_result(
    paths: &[PathBuf],
    destination: PathBuf,
    kind: TransferKind,
    clipboard_paste_token: Option<u64>,
    outcome: DispatchOutcome,
) -> Option<FileOperationResult> {
    if outcome == DispatchOutcome::NotStarted {
        return None;
    }
    let mut folders = HashSet::from([destination]);
    if kind == TransferKind::Move {
        folders.extend(
            paths
                .iter()
                .filter_map(|path| path.parent().map(Path::to_path_buf)),
        );
    }
    Some(FileOperationResult::ShellTransferReturned {
        parent_folders: folders.into_iter().collect(),
        clipboard_paste_token,
        dispatch_failed: outcome == DispatchOutcome::Failed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_nonempty_filesystem_batches_are_eligible() {
        let destination = Path::new(r"D:\Destination");
        assert!(is_filesystem_transfer(
            &[PathBuf::from(r"C:\Source\file.txt")],
            destination
        ));
        assert!(!is_filesystem_transfer(&[], destination));
        assert!(!is_filesystem_transfer(
            &[PathBuf::from("relative.txt")],
            destination
        ));
        assert!(!is_filesystem_transfer(
            &[PathBuf::from("shell:RecycleBinFolder")],
            destination
        ));
        assert!(!is_filesystem_transfer(
            &[PathBuf::from(r"C:\Source\file.txt")],
            Path::new("shell:RecycleBinFolder")
        ));
    }

    #[test]
    fn shell_preparation_failure_is_the_only_fallback_case() {
        let paths = [PathBuf::from(r"C:\Source\file.txt")];
        assert!(handoff_result(
            &paths,
            PathBuf::from(r"D:\Destination"),
            TransferKind::Move,
            Some(4),
            DispatchOutcome::NotStarted
        )
        .is_none());
        for outcome in [DispatchOutcome::Returned, DispatchOutcome::Failed] {
            let result = handoff_result(
                &paths,
                PathBuf::from(r"D:\Destination"),
                TransferKind::Move,
                Some(4),
                outcome,
            )
            .unwrap();
            let FileOperationResult::ShellTransferReturned {
                parent_folders,
                clipboard_paste_token,
                dispatch_failed,
            } = result
            else {
                panic!("A handoff must never emit a transfer-completed result");
            };
            assert_eq!(clipboard_paste_token, Some(4));
            assert_eq!(dispatch_failed, outcome == DispatchOutcome::Failed);
            assert!(parent_folders.contains(&PathBuf::from(r"C:\Source")));
            assert!(parent_folders.contains(&PathBuf::from(r"D:\Destination")));
        }
    }
}
