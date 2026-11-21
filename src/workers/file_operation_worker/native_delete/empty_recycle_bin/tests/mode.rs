use super::*;
use std::cell::Cell;
use windows::Win32::Foundation::HWND;

fn request() -> FileOperationRequest {
    FileOperationRequest::EmptyRecycleBin {
        hwnd: SendHwnd(HWND::default()),
    }
}

#[test]
fn empty_recycle_bin_engine_preference_selects_ui_for_each_request() {
    let runtime = Arc::new(CopyRuntime::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    // Also cover toggling the preference without restarting the runtime.
    for enabled in [false, true, false] {
        runtime.enabled.store(enabled, Ordering::Relaxed);
        let themed_called = Cell::new(false);
        let shell_called = Cell::new(false);
        assert!(try_handle_with(
            &request(),
            &sender,
            &runtime,
            |_| {
                assert!(runtime.empty_recycle_bin_running.load(Ordering::Acquire));
                themed_called.set(true);
            },
            |_| {
                assert!(runtime.empty_recycle_bin_running.load(Ordering::Acquire));
                shell_called.set(true);
            },
        ));
        assert_eq!(themed_called.get(), enabled);
        assert_eq!(shell_called.get(), !enabled);
        assert!(!runtime.empty_recycle_bin_running.load(Ordering::Acquire));
        assert!(runtime.jobs().is_empty());
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::RecycleBinChanged
        ));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn empty_recycle_bin_shell_mode_reports_result_without_creating_app_ui() {
    for result in [S_OK, E_ABORT, E_ACCESSDENIED] {
        let runtime = Arc::new(CopyRuntime::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        let calls = Cell::new(0);
        assert!(try_handle_with(
            &request(),
            &sender,
            &runtime,
            |_| panic!("Disabled engine must not open themed UI"),
            |hwnd| {
                handle_shell(hwnd, &sender, |_| {
                    calls.set(calls.get() + 1);
                    result.ok()
                })
            },
        ));
        assert_eq!(calls.get(), 1);
        assert!(runtime.jobs().is_empty());
        if result.is_err() {
            let FileOperationResult::OperationFailed { message } = receiver.try_recv().unwrap()
            else {
                panic!("Expected native operation error");
            };
            assert_eq!(
                message,
                if result == E_ABORT {
                    rust_i18n::t!("operations.error_cancelled").to_string()
                } else {
                    tracking::error_message(&windows::core::Error::from_hresult(result))
                }
            );
        }
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::RecycleBinChanged
        ));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn empty_recycle_bin_simultaneous_requests_are_ignored_in_both_modes() {
    for enabled in [false, true] {
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(enabled, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let _running = Running::acquire(&runtime).unwrap();
        assert!(try_handle_with(
            &request(),
            &sender,
            &runtime,
            |_| panic!("Concurrent themed operation"),
            |_| panic!("Concurrent native operation"),
        ));
        assert!(runtime.jobs().is_empty());
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn empty_recycle_bin_preference_change_during_execution_does_not_run_another_operation() {
    for enabled in [false, true] {
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(enabled, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let calls = Cell::new(0);
        let operation = |_: SendHwnd| {
            calls.set(calls.get() + 1);
            runtime.enabled.store(!enabled, Ordering::Relaxed);
        };
        assert!(try_handle_with(
            &request(),
            &sender,
            &runtime,
            &operation,
            &operation,
        ));
        assert_eq!(calls.get(), 1);
        assert_eq!(runtime.enabled.load(Ordering::Relaxed), !enabled);
        assert!(!runtime.empty_recycle_bin_running.load(Ordering::Acquire));
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::RecycleBinChanged
        ));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn empty_recycle_bin_dispatch_refreshes_and_releases_guard_on_panic_in_both_modes() {
    for enabled in [false, true] {
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(enabled, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            try_handle_with(
                &request(),
                &sender,
                &runtime,
                |_| panic!("simulated themed operation panic"),
                |_| panic!("simulated native operation panic"),
            )
        }));
        assert!(result.is_err());
        assert!(!runtime.empty_recycle_bin_running.load(Ordering::Acquire));
        assert!(Running::acquire(&runtime).is_some());
        assert!(matches!(
            receiver.try_recv().unwrap(),
            FileOperationResult::RecycleBinChanged
        ));
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn empty_recycle_bin_dispatch_leaves_unrelated_requests_unchanged_in_both_modes() {
    for enabled in [false, true] {
        let runtime = Arc::new(CopyRuntime::default());
        runtime.enabled.store(enabled, Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        let request = FileOperationRequest::ShowProperties {
            paths: Vec::new(),
            hwnd: SendHwnd(HWND::default()),
        };
        assert!(!try_handle_with(
            &request,
            &sender,
            &runtime,
            |_| panic!("Unrelated request entered themed emptying"),
            |_| panic!("Unrelated request entered native emptying"),
        ));
        assert!(!runtime.empty_recycle_bin_running.load(Ordering::Acquire));
        assert!(runtime.jobs().is_empty());
        assert!(receiver.try_recv().is_err());
    }
}
