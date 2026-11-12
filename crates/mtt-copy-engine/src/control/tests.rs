use super::*;
use std::sync::{mpsc, Arc, Barrier};

fn wait_until_paused(progress: &Progress) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let state = progress
            .control
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.participants > 0 && state.waiting == state.participants {
            return true;
        }
        drop(state);
        std::thread::yield_now();
    }
    false
}

#[test]
fn pause_acknowledges_all_workers_and_resume_releases_them() {
    let progress = Arc::new(Progress::default());
    let barrier = Arc::new(Barrier::new(3));
    let (ready, ready_rx) = mpsc::channel();
    let (done, done_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..2 {
            let progress = progress.clone();
            let barrier = barrier.clone();
            let ready = ready.clone();
            let done = done.clone();
            scope.spawn(move || {
                let _participant = progress.participant();
                ready.send(()).unwrap();
                barrier.wait();
                done.send(progress.checkpoint().is_ok()).unwrap();
            });
        }
        for _ in 0..2 {
            ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        }
        progress.pause();
        assert_eq!(progress.pause_state(), PauseState::Pausing);
        barrier.wait();
        let paused = wait_until_paused(&progress);
        let blocked = done_rx.try_recv().is_err();
        progress.resume();
        for _ in 0..2 {
            assert!(done_rx.recv_timeout(Duration::from_secs(3)).unwrap());
        }
        assert!(paused && blocked);
        assert_eq!(progress.pause_state(), PauseState::Running);
    });
}

#[test]
fn cancelling_a_paused_worker_wakes_it_without_resuming() {
    let progress = Arc::new(Progress::default());
    progress.pause();
    let (done, done_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let worker = progress.clone();
        scope.spawn(move || {
            let _participant = worker.participant();
            done.send(worker.checkpoint().unwrap_err().kind()).unwrap();
        });
        let paused = wait_until_paused(&progress);
        progress.cancel();
        assert_eq!(
            done_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            io::ErrorKind::Interrupted
        );
        assert!(paused);
    });
}

#[test]
fn cancellation_racing_the_pause_wait_never_misses_a_wakeup() {
    for _ in 0..32 {
        let progress = Arc::new(Progress::default());
        progress.pause();
        let worker = progress.clone();
        let (done, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let _participant = worker.participant();
            done.send(worker.checkpoint().is_err()).unwrap();
        });
        progress.cancel();
        assert!(receiver.recv_timeout(Duration::from_secs(3)).unwrap());
        thread.join().unwrap();
    }
}

#[test]
fn active_clock_excludes_completed_and_ongoing_pauses() {
    let progress = Progress::default();
    let now = Instant::now();
    {
        let mut state = progress.control.state.lock().unwrap();
        state.started = now - Duration::from_secs(30);
        state.paused_time = Duration::from_secs(5);
        state.requested_at = Some(now - Duration::from_secs(10));
    }
    assert_eq!(progress.active_elapsed(), Duration::from_secs(15));
    assert_eq!(progress.active_elapsed(), Duration::from_secs(15));
    progress.resume();
    assert!(progress.active_elapsed() >= Duration::from_secs(15));
}

#[test]
fn preparation_can_pause_before_any_destination_is_written() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("file.txt");
    let target = root.path().join("target");
    std::fs::write(&source, b"contents").unwrap();
    std::fs::create_dir(&target).unwrap();
    let progress = Arc::new(Progress::default());
    progress.pause();
    let (done, done_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let worker = progress.clone();
        let destination = target.clone();
        scope.spawn(move || {
            done.send(crate::copy(&[source], &destination, &worker))
                .unwrap();
        });
        let paused = wait_until_paused(&progress);
        let empty = std::fs::read_dir(&target).unwrap().count() == 0;
        progress.resume();
        let summary = done_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .unwrap();
        assert!(paused && empty);
        assert_eq!(summary.files_copied, 1);
        assert_eq!(std::fs::read(target.join("file.txt")).unwrap(), b"contents");
    });
}
