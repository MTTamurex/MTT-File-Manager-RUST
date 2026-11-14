use super::*;
use crate::infrastructure::copy_engine::CopyRuntime;
use std::sync::atomic::Ordering;

#[test]
fn successful_nonzero_results_count_recycling_once_and_never_count_retained_sources() {
    for code in [
        HRESULT(0),
        COPYENGINE_S_DONT_PROCESS_CHILDREN,
        COPYENGINE_S_ALREADY_DONE,
        COPYENGINE_S_COLLISIONRESOLVED,
    ] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("file.txt");
        let target = root.path().join("recycled.txt");
        std::fs::write(&source, b"contents").unwrap();
        let progress = DeleteProgress::new(false);
        let completion = Completion::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observer: RemovalObserver = {
            let seen = seen.clone();
            Arc::new(move |path| seen.lock().unwrap().push(path.to_owned()))
        };
        completion.begin(Some(1), Some(source.clone()));
        completion.record(Some(1), None, code, true, &progress, &observer);
        assert_eq!(progress.removed.load(Ordering::Relaxed), 0);
        // Simulate Shell publication in a disposable tree, never the real bin.
        std::fs::rename(&source, &target).unwrap();
        completion.flush(&progress, &observer);
        completion.record(
            Some(1),
            Some(source.clone()),
            code,
            true,
            &progress,
            &observer,
        );
        completion.flush(&progress, &observer);
        assert_eq!(progress.removed.load(Ordering::Relaxed), 1);
        assert_eq!(progress.recycled.load(Ordering::Relaxed), 1);
        assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 0);
        assert_eq!(*seen.lock().unwrap(), vec![source]);
    }
}

#[test]
fn failed_skipped_and_pending_results_never_count_as_completed() {
    for code in [
        windows::Win32::Foundation::S_FALSE,
        E_ABORT,
        COPYENGINE_E_USER_CANCELLED,
        COPYENGINE_E_RECYCLE_FORCE_NUKE,
        COPYENGINE_S_USER_IGNORED,
        COPYENGINE_S_PENDING,
        COPYENGINE_S_PENDING_DELETE,
        COPYENGINE_S_NOT_HANDLED,
        COPYENGINE_S_USER_RETRY,
    ] {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("file.txt");
        let completion = Completion::default();
        let progress = DeleteProgress::new(false);
        let observer: RemovalObserver = Arc::new(|_| panic!("No completed removal"));
        completion.begin(Some(1), Some(missing));
        completion.record(Some(1), None, code, true, &progress, &observer);
        completion.flush(&progress, &observer);
        assert_eq!(progress.removed.load(Ordering::Relaxed), 0);
        assert_eq!(progress.recycled.load(Ordering::Relaxed), 0);
    }
}

fn shell_item(path: &std::path::Path) -> IShellItem {
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None) }.unwrap()
}

#[test]
fn actual_sink_callbacks_count_a_recycled_item_after_source_disappears_even_on_cancellation() {
    let _com = crate::infrastructure::windows::ComScope::sta();
    for cancel in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("file.txt");
        let target = root.path().join("recycled.txt");
        std::fs::write(&source, b"contents").unwrap();
        let original = shell_item(&source);
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![source.clone()], false);
        let completion = Arc::new(Completion::default());
        let sink: IFileOperationProgressSink = super::super::Sink {
            operation: job.operation.clone(),
            removed: Arc::new(|_| {}),
            unavailable: Arc::new(AtomicBool::new(false)),
            completed: completion.clone(),
        }
        .into();
        unsafe { sink.PreDeleteItem(TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32, &original) }.unwrap();
        std::fs::rename(&source, &target).unwrap();
        let recycled = shell_item(&target);
        if cancel {
            job.operation.progress.cancel();
        }
        let result = unsafe {
            sink.PostDeleteItem(
                TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32,
                &original,
                COPYENGINE_S_DONT_PROCESS_CHILDREN,
                &recycled,
            )
        };
        assert_eq!(result.is_err(), cancel);
        let progress = job.operation.deletion.as_ref().unwrap();
        assert_eq!(progress.recycled.load(Ordering::Relaxed), 1);
        assert_eq!(progress.removed.load(Ordering::Relaxed), 1);
        assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 0);
        assert_eq!(progress.failures.load(Ordering::Relaxed), 0);
        assert_eq!(std::fs::read(&target).unwrap(), b"contents");
    }
}

#[test]
fn successful_removal_without_a_recycled_item_is_reported_as_permanent() {
    let root = tempfile::tempdir().unwrap();
    let progress = DeleteProgress::new(false);
    let completion = Completion::default();
    let observer: RemovalObserver = Arc::new(|_| {});
    completion.begin(Some(1), Some(root.path().join("file.txt")));
    completion.record(Some(1), None, HRESULT(0), false, &progress, &observer);
    assert_eq!(progress.permanently_removed.load(Ordering::Relaxed), 1);
    assert_eq!(progress.recycled.load(Ordering::Relaxed), 0);
    assert_eq!(progress.removed.load(Ordering::Relaxed), 1);
}
