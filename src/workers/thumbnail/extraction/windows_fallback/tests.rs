use super::*;
use std::cell::Cell;

fn pixels() -> Option<(Vec<u8>, u32, u32)> {
    Some((vec![1, 2, 3, 255], 1, 1))
}

#[test]
fn shell_precedes_media_foundation_and_success_stops_the_chain() {
    let result = run(pixels, || panic!("Shell succeeded"), || None);
    assert!(matches!(result, Outcome::Success(_)));
    let order = Cell::new(0);
    let result = run(
        || {
            order.set(1);
            None
        },
        || {
            assert_eq!(order.get(), 1);
            order.set(2);
            pixels()
        },
        || None,
    );
    assert!(matches!(result, Outcome::Success(_)));
    assert_eq!(order.get(), 2);
    assert!(matches!(run(|| None, || None, || None), Outcome::Failed));
}

#[test]
fn cancellation_between_windows_stages_prevents_more_work_and_late_success() {
    for shell_succeeds in [false, true] {
        let cancelled = Cell::new(false);
        let result = run(
            || {
                cancelled.set(true);
                if shell_succeeds {
                    pixels()
                } else {
                    None
                }
            },
            || panic!("Cancelled before MF"),
            || cancelled.get().then_some(Outcome::Cancelled),
        );
        assert!(matches!(result, Outcome::Cancelled));
    }
    let cancelled = Cell::new(false);
    let result = run(
        || None,
        || {
            cancelled.set(true);
            pixels()
        },
        || cancelled.get().then_some(Outcome::Cancelled),
    );
    assert!(matches!(result, Outcome::Cancelled));
}

#[test]
fn unsafe_or_cancelled_source_stops_before_shell() {
    let result = run(
        || panic!("Unsafe source"),
        || panic!("Unsafe source"),
        || Some(Outcome::UnsafeToRead(FileReadSafety::RecentlyChanging)),
    );
    assert!(matches!(
        result,
        Outcome::UnsafeToRead(FileReadSafety::RecentlyChanging)
    ));
    let result = run(
        || panic!("Cancelled"),
        || panic!("Cancelled"),
        || Some(Outcome::Cancelled),
    );
    assert!(matches!(result, Outcome::Cancelled));
}
