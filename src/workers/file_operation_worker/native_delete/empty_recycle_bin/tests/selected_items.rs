use super::*;
use windows::Win32::Foundation::HWND;

fn request(paths: Vec<std::path::PathBuf>) -> FileOperationRequest {
    FileOperationRequest::DeletePermanently {
        physical_paths: paths,
        hwnd: crate::workers::file_operation_worker::SendHwnd(HWND::default()),
    }
}

#[test]
fn selected_item_handler_only_intercepts_enabled_recycle_bin_requests() {
    let runtime = Arc::new(CopyRuntime::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    let selected = request(vec![std::path::PathBuf::from(r"C:\$Recycle.Bin\entry")]);
    assert!(!super::super::selected_items::try_handle(
        &selected, &sender, &runtime
    ));
    runtime.enabled.store(true, Ordering::Relaxed);
    let ordinary = request(vec![std::path::PathBuf::from(r"C:\folder\file.txt")]);
    assert!(!super::super::selected_items::try_handle(
        &ordinary, &sender, &runtime
    ));
    assert!(runtime.jobs().is_empty());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn selected_item_handler_rejects_concurrent_recycle_bin_operations() {
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, receiver) = std::sync::mpsc::channel();
    let _running = Running::acquire(&runtime).unwrap();
    let selected = request(vec![std::path::PathBuf::from(r"C:\$Recycle.Bin\entry")]);
    let FileOperationRequest::DeletePermanently { physical_paths, .. } = &selected else {
        unreachable!();
    };
    assert!(super::super::selected_items::is_recycle_bin_request(
        physical_paths
    ));
    assert!(super::super::selected_items::try_handle(
        &selected, &sender, &runtime,
    ));
    assert!(runtime.jobs().is_empty());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn selected_item_confirmation_uses_custom_confirmation_and_selected_count() {
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete_with_label(Vec::new(), true, None);
    let operation = job.operation.clone();
    let worker =
        std::thread::spawn(move || confirmation::ask_selected_recycle_bin_items(&operation, 2));
    let request =
        crate::infrastructure::delete_engine::confirmation::tests::wait_for_request(&job.operation);
    assert_eq!(request.kind, confirmation::Kind::SelectedRecycleBinItems);
    assert_eq!(request.total, 2);
    assert!(request.paths.is_empty());
    assert!(job
        .operation
        .deletion
        .as_ref()
        .unwrap()
        .confirmation
        .respond(request.id, false));
    assert!(!worker.join().unwrap());
}

#[test]
fn keyboard_and_permanent_delete_dispatch_accept_physical_and_namespace_bin_children() {
    let runtime = Arc::new(CopyRuntime::default());
    runtime.enabled.store(true, Ordering::Relaxed);
    let (sender, receiver) = std::sync::mpsc::channel();
    // Prevent any real enumeration or deletion. Reaching this guard proves
    // the public dispatcher routed both UI request types into the bin handler.
    let _running = Running::acquire(&runtime).unwrap();
    for identity in [
        r"C:\$Recycle.Bin\entry",
        r"::{645FF040-5081-101B-9F08-00AA002F954E}\entry",
        r"\\?\::{645FF040-5081-101B-9F08-00AA002F954E}\entry",
    ] {
        for request in [
            FileOperationRequest::delete(vec![identity.into()], HWND::default()),
            FileOperationRequest::delete_permanently(vec![identity.into()], HWND::default()),
        ] {
            assert!(
                crate::workers::file_operation_worker::native_delete::try_handle(
                    &request, &sender, &runtime,
                )
            );
        }
    }
    assert!(runtime.jobs().is_empty());
    assert!(receiver.try_recv().is_err());
}
