use super::*;
use crate::profile::{choose, Device, Kind};
use std::sync::{mpsc, Arc};
use std::time::Instant;

fn policy() -> Policy {
    let ssd = Device {
        disk: Some(1),
        kind: Kind::Ssd,
    };
    choose(&[ssd], ssd, Some(0))
}

#[test]
fn large_limit_is_shared_between_jobs_and_cancellation_wakes_waiters() {
    let gate = Gate::default();
    let policy = policy();
    let first = Progress::default();
    let second = Progress::default();
    let permit = gate.acquire(&policy, true, &first).unwrap();
    std::thread::scope(|scope| {
        let (done, receiver) = mpsc::channel();
        let gate = &gate;
        let policy = &policy;
        let second = &second;
        scope.spawn(move || {
            done.send(gate.acquire(policy, true, second).is_none())
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while second.telemetry.waiting.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        second.cancel();
        assert!(receiver.recv_timeout(Duration::from_secs(3)).unwrap());
    });
    drop(permit);
    assert_eq!(first.telemetry.active.load(Ordering::Relaxed), 0);
    assert_eq!(second.telemetry.waiting.load(Ordering::Relaxed), 0);
    assert!(gate.counts.lock().unwrap().is_empty());
}

#[test]
fn paused_transfers_release_the_slot_and_reacquire_before_resuming() {
    let gate = Arc::new(Gate::default());
    let policy = policy();
    let progress = Progress::default();
    let other = Progress::default();
    let (ready, ready_rx) = mpsc::channel();
    let (checkpoint, go) = mpsc::channel();
    let (done, done_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let gate = &gate;
        let policy = &policy;
        let progress = &progress;
        scope.spawn(move || {
            let _participant = progress.participant();
            let permit = gate.acquire(policy, true, progress).unwrap();
            ready.send(()).unwrap();
            go.recv_timeout(Duration::from_secs(3)).unwrap();
            done.send(permit.checkpoint().is_ok()).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        progress.pause();
        checkpoint.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while progress.pause_state() != crate::PauseState::Paused && Instant::now() < deadline {
            std::thread::yield_now();
        }
        let paused = progress.pause_state() == crate::PauseState::Paused;
        // Use a cancelling fallback so a broken admission policy cannot hang this test.
        if !paused {
            progress.cancel();
        }
        assert!(paused);
        let (acquired, acquired_rx) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        let other = &other;
        scope.spawn(move || {
            let permit = gate.acquire(policy, true, other);
            acquired.send(permit.is_some()).unwrap();
            if permit.is_some() {
                let _ = release_rx.recv_timeout(Duration::from_secs(3));
            }
        });
        let admitted = acquired_rx
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or(false);
        if !admitted {
            other.cancel();
            progress.cancel();
        }
        assert!(admitted, "Paused transfer retained its disk slot");
        let released_slot = progress.telemetry.active.load(Ordering::Relaxed) == 0;
        progress.resume();
        let resumed_early = done_rx.recv_timeout(Duration::from_millis(30)).ok();
        release.send(()).unwrap();
        let finished =
            resumed_early.unwrap_or_else(|| done_rx.recv_timeout(Duration::from_secs(3)).unwrap());
        assert!(released_slot);
        assert!(
            resumed_early.is_none(),
            "Transfer resumed without reacquiring its disk slot"
        );
        assert!(finished);
    });
    assert!(gate.counts.lock().unwrap().is_empty());
}

#[test]
fn small_files_can_use_the_remaining_budget_but_all_workers_cannot_copy_large_files() {
    let gate = Gate::default();
    let policy = policy();
    let progress = Progress::default();
    let large = gate.acquire(&policy, true, &progress).unwrap();
    let small = gate.acquire(&policy, false, &progress).unwrap();
    assert_eq!(progress.telemetry.active.load(Ordering::Relaxed), 2);
    assert_eq!(gate.counts.lock().unwrap()[&Resource::Disk(1)].large, 1);
    drop((large, small));
    assert_eq!(progress.telemetry.active.load(Ordering::Relaxed), 0);
}

#[test]
fn another_jobs_looser_policy_cannot_bypass_an_active_stricter_disk_limit() {
    let gate = Gate::default();
    let strict = policy();
    let loose = choose(
        &[Device {
            disk: Some(1),
            kind: Kind::Ssd,
        }],
        Device {
            disk: Some(2),
            kind: Kind::Ssd,
        },
        Some(0),
    );
    let first = Progress::default();
    let second = Progress::default();
    let permit = gate.acquire(&strict, true, &first).unwrap();
    std::thread::scope(|scope| {
        let (done, receiver) = mpsc::channel();
        let second = &second;
        let gate = &gate;
        let loose = &loose;
        scope.spawn(move || {
            done.send(gate.acquire(loose, true, second).is_none())
                .unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while second.telemetry.waiting.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        second.cancel();
        assert!(receiver.recv_timeout(Duration::from_secs(3)).unwrap());
    });
    drop(permit);
    assert!(gate.counts.lock().unwrap().is_empty());
}
