use super::{
    sanitize_operation_path, sanitize_operation_paths, FileOperationRequest, FileOperationResult,
};
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome, OutcomeKind};
use std::sync::atomic::Ordering;
use std::sync::{mpsc::Sender, Arc};

pub(super) fn try_handle(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
    runtime: &Arc<CopyRuntime>,
) -> bool {
    if !runtime.enabled.load(Ordering::Relaxed) {
        return false;
    }
    let (paths, destination) = match request {
        FileOperationRequest::Copy {
            path, dest_folder, ..
        } => (std::slice::from_ref(path), dest_folder),
        FileOperationRequest::CopyBatch {
            paths, dest_folder, ..
        } => (paths.as_slice(), dest_folder),
        _ => return false,
    };
    // Whole mixed batches retain their existing extraction/Shell semantics.
    if !super::shell_transfers::is_filesystem_transfer(paths, destination) {
        return false;
    }
    let validated = sanitize_operation_paths(paths).and_then(|paths| {
        sanitize_operation_path(destination).map(|destination| (paths, destination))
    });
    let job = runtime.start(paths.to_vec(), destination.clone());
    let _monitor =
        crate::infrastructure::copy_engine::diagnostics::Monitor::start(job.operation.clone());
    let result = match validated {
        Ok((paths, destination)) => {
            mtt_copy_engine::copy(&paths, &destination, &job.operation.progress)
        }
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "Copy path validation failed",
        )),
    };
    // Refresh even after partial failure/cancellation. Never retry an attempted
    // native job through Shell: some destination files may already exist.
    let copied_dests = result
        .as_ref()
        .map(|summary| summary.destinations.clone())
        .unwrap_or_default();
    let _ = sender.send(FileOperationResult::CopyCompleted {
        dest_folder: destination.clone(),
        copied_dests,
    });
    let kind = match &result {
        Ok(summary) if summary.cancelled => OutcomeKind::Cancelled,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => OutcomeKind::Cancelled,
        Ok(summary) if summary.failures == 0 => OutcomeKind::Completed,
        _ => OutcomeKind::Failed,
    };
    let message = match &result {
        Ok(summary) if summary.cancelled => {
            Some(rust_i18n::t!("copy_engine.cancelled").to_string())
        }
        Ok(summary) if summary.failures > 0 => Some(format!(
            "{}: {}",
            rust_i18n::t!("copy_engine.failed", count = summary.failures),
            summary
                .first_error
                .as_ref()
                .map(|error| error.to_string())
                .unwrap_or_default()
        )),
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
            Some(rust_i18n::t!("copy_engine.cancelled").to_string())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Some(rust_i18n::t!("copy_engine.conflict").to_string())
        }
        Err(error) => Some(format!("{}: {}", rust_i18n::t!("copy_engine.error"), error)),
    };
    job.finish(Outcome { kind, message });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn default_runtime_does_not_intercept_copy() {
        let runtime = Arc::new(CopyRuntime::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        let request = FileOperationRequest::copy_batch(
            vec![PathBuf::from(r"C:\source\file.txt")],
            PathBuf::from(r"D:\target"),
            windows::Win32::Foundation::HWND::default(),
        );
        assert!(!try_handle(&request, &sender, &runtime));
        assert!(receiver.try_recv().is_err());
        assert!(runtime.jobs().is_empty());
    }

    #[test]
    fn enabled_runtime_leaves_moves_deletes_and_virtual_items_unchanged() {
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(true, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let paths = vec![PathBuf::from(r"C:\source\file.txt")];
        let destination = PathBuf::from(r"D:\target");
        let hwnd = windows::Win32::Foundation::HWND::default();
        assert!(!try_handle(
            &FileOperationRequest::move_batch(paths.clone(), destination.clone(), hwnd),
            &sender,
            &runtime
        ));
        assert!(!try_handle(
            &FileOperationRequest::delete(paths, hwnd),
            &sender,
            &runtime
        ));
        assert!(!try_handle(
            &FileOperationRequest::copy_batch(
                vec![PathBuf::from("shell:Downloads")],
                destination,
                hwnd
            ),
            &sender,
            &runtime
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn enabled_copy_completes_and_conflict_is_not_retried() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("file.txt");
        let destination = root.path().join("target");
        std::fs::write(&source, b"source").unwrap();
        std::fs::create_dir(&destination).unwrap();
        let request = FileOperationRequest::copy_batch(
            vec![source.clone()],
            destination.clone(),
            windows::Win32::Foundation::HWND::default(),
        );
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(true, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(try_handle(&request, &sender, &runtime));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::CopyCompleted { .. }
        ));
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            std::fs::read(destination.join("file.txt")).unwrap(),
            b"source"
        );
        std::fs::write(destination.join("file.txt"), b"target").unwrap();
        assert!(try_handle(&request, &sender, &runtime));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::CopyCompleted { .. }
        ));
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            runtime.jobs()[1].1.outcome().unwrap().kind,
            OutcomeKind::Failed
        );
        assert_eq!(
            std::fs::read(destination.join("file.txt")).unwrap(),
            b"target"
        );
        assert_eq!(std::fs::read(source).unwrap(), b"source");
        assert_eq!(runtime.jobs().len(), 2);
    }

    #[test]
    fn mixed_archive_batch_stays_on_existing_extraction_path() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("archive.zip");
        let regular = root.path().join("file.txt");
        let destination = root.path().join("target");
        std::fs::write(&archive, b"fixture").unwrap();
        std::fs::write(&regular, b"source").unwrap();
        std::fs::create_dir(&destination).unwrap();
        let request = FileOperationRequest::copy_batch(
            vec![regular, archive.join("entry.txt")],
            destination.clone(),
            windows::Win32::Foundation::HWND::default(),
        );
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(true, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        assert!(!try_handle(&request, &sender, &runtime));
        assert!(receiver.try_recv().is_err());
        assert_eq!(std::fs::read_dir(destination).unwrap().count(), 0);
    }
}
