use super::*;

fn visible() -> WindowState {
    WindowState {
        visible: true,
        focused: true,
        minimized: false,
    }
}

fn begin(recorder: &mut Recorder, now: Instant, state: WindowState) {
    assert!(recorder
        .record(1, now, state, false, None, false, vec![])
        .is_none());
    assert!(recorder
        .record(1, now + IDLE_GRACE, state, false, None, false, vec![])
        .is_none());
}

#[test]
fn minimized_logic_reports_zero_ui_passes_without_waking_the_ui() {
    let mut recorder = Recorder::default();
    let now = Instant::now();
    let state = WindowState {
        visible: false,
        focused: false,
        minimized: true,
    };
    begin(&mut recorder, now, state);
    let report = recorder
        .record(
            1,
            now + IDLE_GRACE + SAMPLE_INTERVAL,
            state,
            false,
            None,
            false,
            vec![],
        )
        .unwrap();
    assert_eq!(report.ui_passes, 0);
    assert_eq!(report.logic_ticks, 1);
}

#[test]
fn repeated_pass_is_counted_once_and_state_changes_discard_old_samples() {
    let mut recorder = Recorder::default();
    let now = Instant::now();
    begin(&mut recorder, now, visible());
    let cause = egui::RepaintCause {
        file: "src/ui/clipboard_feedback.rs",
        line: 42,
        reason: "".into(),
    };
    for _ in 0..2 {
        recorder.record(
            1,
            now + IDLE_GRACE,
            visible(),
            false,
            Some(10),
            true,
            vec![cause.clone()],
        );
    }
    let report = recorder
        .record(
            1,
            now + IDLE_GRACE + SAMPLE_INTERVAL,
            visible(),
            false,
            Some(10),
            false,
            vec![],
        )
        .unwrap();
    assert_eq!(report.ui_passes, 1);
    assert_eq!(report.zero_previous_delay_passes, 1);
    assert_eq!(report.causes, vec![("clipboard_feedback", 42, 1)]);
    let background = WindowState {
        focused: false,
        ..visible()
    };
    assert!(recorder
        .record(
            1,
            now + Duration::from_secs(13),
            background,
            false,
            None,
            false,
            vec![]
        )
        .is_none());
    assert_eq!(recorder.ui_passes, 0);
    assert!(recorder.sample_started.is_none());
}

#[test]
fn input_and_new_session_reset_sample_and_idle_grace() {
    let mut recorder = Recorder::default();
    let now = Instant::now();
    begin(&mut recorder, now, visible());
    recorder.record(
        1,
        now + Duration::from_secs(3),
        visible(),
        false,
        Some(1),
        false,
        vec![],
    );
    recorder.record(
        1,
        now + Duration::from_secs(4),
        visible(),
        true,
        None,
        false,
        vec![],
    );
    assert_eq!(recorder.ui_passes, 0);
    assert!(recorder.sample_started.is_none());
    recorder.record(
        3,
        now + Duration::from_secs(5),
        visible(),
        false,
        None,
        false,
        vec![],
    );
    assert_eq!(recorder.session, Some(3));
    assert_eq!(recorder.last_input, Some(now + Duration::from_secs(5)));
}

#[test]
fn cause_storage_is_bounded() {
    let mut recorder = Recorder::default();
    let now = Instant::now();
    begin(&mut recorder, now, visible());
    let causes = (0..200)
        .map(|line| egui::RepaintCause {
            file: "unknown",
            line,
            reason: "".into(),
        })
        .collect();
    recorder.record(
        1,
        now + IDLE_GRACE,
        visible(),
        false,
        Some(1),
        false,
        causes,
    );
    assert_eq!(recorder.causes.len(), MAX_CAUSE_KEYS);
    assert_eq!(recorder.causes_overflow, 200 - MAX_CAUSE_KEYS as u64);
}

#[test]
fn visible_sample_finishes_after_ui_and_resets_both_pass_counters() {
    let mut recorder = Recorder::default();
    let now = Instant::now();
    let end = now + IDLE_GRACE + SAMPLE_INTERVAL;
    begin(&mut recorder, now, visible());
    recorder.record(1, now + IDLE_GRACE, visible(), false, Some(1), true, vec![]);
    assert!(recorder
        .record(1, end, visible(), false, None, false, vec![])
        .is_none());
    let report = recorder
        .record(1, end, visible(), false, Some(2), false, vec![])
        .unwrap();
    assert_eq!(report.ui_passes, 2);
    assert_eq!(report.zero_previous_delay_passes, 1);
    assert_eq!(report.logic_ticks, 1);
    assert_eq!(report.elapsed, SAMPLE_INTERVAL);
    assert_eq!(recorder.ui_passes, 0);
    assert_eq!(recorder.zero_previous_delay_passes, 0);

    let next = recorder
        .record(
            1,
            end + SAMPLE_INTERVAL,
            visible(),
            false,
            Some(3),
            false,
            vec![],
        )
        .unwrap();
    assert_eq!(next.ui_passes, 1);
    assert_eq!(next.zero_previous_delay_passes, 0);
}

#[test]
fn input_resets_zero_delay_counter_and_hidden_state_discards_visible_sample() {
    let mut recorder = Recorder::default();
    let now = Instant::now();
    begin(&mut recorder, now, visible());
    recorder.record(1, now + IDLE_GRACE, visible(), false, Some(1), true, vec![]);
    assert_eq!(recorder.zero_previous_delay_passes, 1);
    recorder.record(1, now + IDLE_GRACE, visible(), true, None, false, vec![]);
    assert_eq!(recorder.zero_previous_delay_passes, 0);
    begin(&mut recorder, now + IDLE_GRACE, visible());
    recorder.record(
        1,
        now + IDLE_GRACE * 2,
        visible(),
        false,
        Some(2),
        true,
        vec![],
    );
    let hidden = WindowState {
        visible: false,
        ..visible()
    };
    recorder.record(1, now + IDLE_GRACE * 2, hidden, false, None, false, vec![]);
    assert_eq!(recorder.zero_previous_delay_passes, 0);
    assert_eq!(recorder.ui_passes, 0);
}
