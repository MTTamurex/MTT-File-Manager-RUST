use super::*;
use crate::infrastructure::copy_engine::CopyRuntime;

#[test]
fn unavailable_recycling_never_deletes_without_a_separate_permanent_answer() {
    for allow in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.txt");
        std::fs::write(&path, b"synthetic").unwrap();
        let runtime = CopyRuntime::default();
        let job = runtime.start_delete(vec![path.clone()], false);
        let completed = run_with_recycler(
            std::slice::from_ref(&path),
            &job.operation,
            Arc::new(|_| {}),
            &|_, kind, paths| {
                assert_eq!(kind, Kind::RecycleUnavailable);
                assert_eq!(paths, std::slice::from_ref(&path));
                assert!(path.exists());
                allow
            },
            |_, _, _| Ok(recycle::Status::Unavailable),
        );
        assert_eq!(completed, allow);
        assert_eq!(path.exists(), !allow);
        assert_eq!(
            job.operation
                .deletion
                .as_ref()
                .unwrap()
                .used_permanent_fallback
                .load(Ordering::Relaxed),
            allow
        );
        temp.close().unwrap();
    }
}

#[test]
fn fallback_only_applies_to_the_blocked_root_and_cancel_stops_later_roots() {
    let temp = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..3)
        .map(|index| temp.path().join(format!("file-{index}.txt")))
        .collect();
    for path in &paths {
        std::fs::write(path, b"synthetic").unwrap();
    }
    let runtime = CopyRuntime::default();
    let job = runtime.start_delete(paths.clone(), false);
    let completed = run_with_recycler(
        &paths,
        &job.operation,
        Arc::new(|_| {}),
        &|_, kind, pending| {
            assert_eq!(kind, Kind::RecycleUnavailable);
            assert_eq!(pending, &paths[1..2]);
            false
        },
        |path, _, _| {
            if path == &paths[0] {
                std::fs::remove_file(path).unwrap();
                Ok(recycle::Status::Completed)
            } else {
                assert_eq!(path, &paths[1]);
                Ok(recycle::Status::Unavailable)
            }
        },
    );
    assert!(!completed);
    assert!(!paths[0].exists());
    assert!(paths[1].exists());
    assert!(paths[2].exists());
    temp.close().unwrap();
}
