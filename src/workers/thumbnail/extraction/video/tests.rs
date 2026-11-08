use super::*;
use std::cell::Cell;

fn frame() -> Frame {
    Frame {
        rgba: vec![1, 2, 3, 255],
        width: 1,
        height: 1,
    }
}
fn success() -> Outcome {
    Outcome::Success((vec![4, 5, 6, 255], 1, 1))
}

#[test]
fn primary_success_never_calls_windows_or_fallback_checks() {
    let outcome = run(
        || Ok(frame()),
        || panic!("No Windows on success"),
        &|| false,
        || panic!("No fallback checkpoint on success"),
    );
    assert!(matches!(outcome, Outcome::Success((data, 1, 1)) if data == [1, 2, 3, 255]));
}

#[test]
fn decoder_and_supervisor_errors_try_windows_once() {
    for error in [
        Error::Decode,
        Error::Helper(Failure::Decode),
        Error::Unavailable,
        Error::Helper(Failure::Unavailable),
        Error::Transport,
        Error::Timeout,
        Error::Helper(Failure::Timeout),
    ] {
        let calls = Cell::new(0);
        let outcome = run(
            || Err(error),
            || {
                calls.set(calls.get() + 1);
                success()
            },
            &|| false,
            || None,
        );
        assert!(matches!(outcome, Outcome::Success(_)), "{error:?}");
        assert_eq!(calls.get(), 1);
    }
}

#[test]
fn unsafe_cancelled_and_queue_wait_failures_never_try_windows() {
    for error in [
        Error::Cancelled,
        Error::QueueTimeout,
        Error::Helper(Failure::IncompleteDownload),
        Error::Helper(Failure::WriteLocked),
        Error::Helper(Failure::RecentlyChanging),
    ] {
        let outcome = run(
            || Err(error),
            || panic!("No fallback for {error:?}"),
            &|| false,
            || panic!("No fallback checkpoint for {error:?}"),
        );
        match error {
            Error::Cancelled => assert!(matches!(outcome, Outcome::Cancelled)),
            Error::QueueTimeout => assert!(matches!(outcome, Outcome::TransientFailure)),
            _ => assert!(matches!(outcome, Outcome::UnsafeToRead(_))),
        }
    }
}

#[test]
fn failed_windows_does_not_make_supervisor_failure_permanent() {
    for error in [
        Error::Decode,
        Error::Helper(Failure::Decode),
        Error::Transport,
        Error::Unavailable,
        Error::Timeout,
    ] {
        let outcome = run(|| Err(error), || Outcome::Failed, &|| false, || None);
        if matches!(error, Error::Decode | Error::Helper(Failure::Decode)) {
            assert!(matches!(outcome, Outcome::Failed));
        } else {
            assert!(matches!(outcome, Outcome::TransientFailure));
        }
    }
}

#[test]
fn changed_or_deleted_source_prevents_windows_fallback() {
    for check in [
        Outcome::Cancelled,
        Outcome::Failed,
        Outcome::UnsafeToRead(FileReadSafety::WriteLocked),
    ] {
        let outcome = run(
            || Err(Error::Decode),
            || panic!("Source no longer safe"),
            &|| false,
            || Some(check),
        );
        assert!(!matches!(outcome, Outcome::Success(_)));
    }
}

#[test]
fn cancellation_discards_primary_and_fallback_results() {
    let cancelled = Cell::new(false);
    let outcome = run(
        || {
            cancelled.set(true);
            Ok(frame())
        },
        || panic!("Cancelled"),
        &|| cancelled.get(),
        || None,
    );
    assert!(matches!(outcome, Outcome::Cancelled));
    cancelled.set(false);
    let outcome = run(
        || Err(Error::Decode),
        || {
            cancelled.set(true);
            success()
        },
        &|| cancelled.get(),
        || None,
    );
    assert!(matches!(outcome, Outcome::Cancelled));
    let outcome = run(
        || panic!("Already cancelled"),
        || panic!("Already cancelled"),
        &|| true,
        || None,
    );
    assert!(matches!(outcome, Outcome::Cancelled));
}
