use super::*;
use crate::infrastructure::diagnostic_logger::DiagnosticField;

fn seed(started: Instant) {
    TRACE.with(|trace| {
        *trace.borrow_mut() = Some(Trace {
            id: 42,
            session: 1,
            generation: None,
            started,
            frames: 0,
            last_loading: false,
            stages: Vec::new(),
        })
    });
}

fn test_frame() -> Frame {
    let now = Instant::now();
    Frame {
        started: now,
        last: now,
        ui_gap: Duration::ZERO,
        stages: Some(Vec::new()),
    }
}

#[test]
fn expired_trace_does_not_instrument_new_work() {
    seed(Instant::now() - TRACE_WINDOW - Duration::from_secs(1));
    assert!(current_id().is_none());
    assert!(Worker::queued(1).is_none());
}

#[test]
fn stages_are_bounded_and_correlated() {
    seed(Instant::now());
    for _ in 0..MAX_STAGES + 5 {
        drop(Stage::new("test_stage"));
    }
    TRACE.with(|trace| {
        let trace = trace.borrow();
        let trace = trace.as_ref().unwrap();
        assert_eq!(trace.stages.len(), MAX_STAGES);
        assert!(trace.stages.iter().all(|(name, _)| *name == "test_stage"));
    });
    let worker = Worker::queued(7).unwrap();
    assert_eq!(worker.id, 42);
    assert_eq!(worker.generation, 7);
    assert_eq!(worker.session, 1);
}

#[test]
fn old_stage_does_not_leak_into_new_trace() {
    seed(Instant::now());
    let stage = Stage::new("old");
    TRACE.with(|trace| trace.borrow_mut().as_mut().unwrap().id = 43);
    drop(stage);
    TRACE.with(|trace| assert!(trace.borrow().as_ref().unwrap().stages.is_empty()));
}

#[test]
fn frame_drains_stages_and_tracks_loading_transition() {
    seed(Instant::now());
    drop(Stage::new("navigation"));
    test_frame().collect(true, 0, 7).unwrap();
    TRACE.with(|trace| {
        let trace = trace.borrow();
        let trace = trace.as_ref().unwrap();
        assert_eq!(trace.frames, 1);
        assert!(trace.last_loading);
        assert!(trace.stages.is_empty());
    });
    let (_, events) = test_frame().collect(false, 3, 7).unwrap();
    assert!(events[0]
        .1
        .contains(&DiagnosticField::Bool("load_completed", true)));
}

#[test]
fn idle_repaint_after_window_does_not_report_stall() {
    seed(Instant::now() - TRACE_WINDOW - Duration::from_secs(1));
    let mut frame = test_frame();
    frame.ui_gap = Duration::from_secs(60);
    assert!(frame.collect(false, 3, 7).is_none());
    TRACE.with(|trace| assert!(trace.borrow().is_none()));
}

#[test]
fn frame_that_stalls_past_window_is_preserved() {
    let started = Instant::now() - TRACE_WINDOW - Duration::from_secs(1);
    seed(started);
    let mut frame = test_frame();
    frame.started = started;
    let (_, events) = frame.collect(false, 3, 7).unwrap();
    assert!(events[0].1.contains(&DiagnosticField::Bool("slow", true)));
    TRACE.with(|trace| assert!(trace.borrow().is_none()));
}

#[test]
fn completion_in_click_frame_is_recorded() {
    seed(Instant::now());
    let _job = Worker::queued(7).unwrap();
    let (_, events) = test_frame().collect(false, 3, 7).unwrap();
    assert!(events[0]
        .1
        .contains(&DiagnosticField::Bool("load_completed", true)));
}

#[test]
fn unrelated_generation_is_not_attached_to_capture() {
    seed(Instant::now());
    let _job = Worker::queued(7).unwrap();
    assert!(Worker::queued(8).is_none());
    assert!(test_frame().collect(false, 3, 8).is_none());
    TRACE.with(|trace| assert!(trace.borrow().is_none()));
}

#[test]
fn inactive_stage_does_not_read_clock_or_collect_data() {
    TRACE.with(|trace| *trace.borrow_mut() = None);
    let mut stage = Stage::new("inactive");
    assert!(stage.started.is_none());
    stage.lap("also_inactive");
    assert!(stage.started.is_none());
}

#[test]
fn disabled_frame_releases_capture() {
    seed(Instant::now());
    let mut frame = test_frame();
    frame.stages = None;
    assert!(frame.collect(false, 3, 7).is_none());
    TRACE.with(|trace| assert!(trace.borrow().is_none()));
}
