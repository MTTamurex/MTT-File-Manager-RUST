use super::*;
use crate::infrastructure::diagnostic_logger::{DiagnosticField, DiagnosticLogger};
use log::LevelFilter;
use std::time::SystemTime;

fn logger() -> DiagnosticLogger {
    let mut console = env_logger::Builder::new();
    console.filter_level(LevelFilter::Off);
    let mut diagnostic = env_logger::Builder::new();
    diagnostic.filter_level(LevelFilter::Info);
    DiagnosticLogger::new(console.build(), diagnostic.build())
}

fn capture() -> Capture {
    Capture {
        session: 1,
        id: 7,
        started: Instant::now(),
    }
}

#[test]
fn disabled_trace_has_no_clock_or_stage_buffer_and_keeps_work_unaffected() {
    let trace = OpenTrace::default();
    assert!(trace.0.is_none());
    assert!(!trace.is_active());
    let mut stages = trace.stages(Phase::ReadFile);
    stages.lap(Phase::ParsePackage);
    assert!(stages.capture.is_none());
    assert!(stages.recording.is_none());
    trace.mark(Milestone::RendererReady);
}

#[test]
fn consecutive_stages_are_nonoverlapping_and_include_the_last_stage() {
    let started = Instant::now();
    let mut recording = Recording {
        phase: Phase::ReadFile,
        started,
        samples: Vec::new(),
    };
    recording.lap_at(Phase::ParsePackage, started + Duration::from_millis(3));
    recording.lap_at(Phase::LoadFonts, started + Duration::from_millis(11));
    recording.lap_at(recording.phase, started + Duration::from_millis(16));
    assert_eq!(
        recording.samples,
        vec![
            (Phase::ReadFile, Duration::from_millis(3)),
            (Phase::ParsePackage, Duration::from_millis(8)),
            (Phase::LoadFonts, Duration::from_millis(5)),
        ]
    );
}

#[test]
fn timing_fields_cannot_contain_document_data() {
    let fields = fields(
        capture(),
        Phase::LayoutDocument.label(),
        Duration::from_millis(123),
    );
    assert_eq!(fields.len(), 4);
    assert!(fields.iter().all(|field| matches!(
        field,
        DiagnosticField::U64("process_id" | "open_id", _)
            | DiagnosticField::Label("phase", "layout_document")
            | DiagnosticField::DurationMs("duration", 123)
    )));
}

#[test]
fn diagnostic_events_are_written_only_for_the_captured_session() {
    let logger = logger();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("diagnostic.log");
    let capture = capture();
    emit_to(
        &logger,
        capture,
        "opening_stage",
        Phase::ReadFile.label(),
        Duration::ZERO,
    );
    assert!(!path.exists());

    logger
        .writer
        .open(1, path.clone(), SystemTime::now())
        .unwrap();
    logger.capture_session.store(1, Ordering::Release);
    emit_to(
        &logger,
        capture,
        "opening_stage",
        Phase::ReadFile.label(),
        Duration::from_millis(25),
    );
    logger.disable_file_logging();
    logger
        .writer
        .open(3, path.clone(), SystemTime::now())
        .unwrap();
    logger.capture_session.store(3, Ordering::Release);
    emit_to(
        &logger,
        capture,
        "stale_event",
        Phase::ParsePackage.label(),
        Duration::ZERO,
    );
    emit_to(
        &logger,
        Capture {
            session: 3,
            ..capture
        },
        "opening_milestone",
        Milestone::FirstPageUiDrawn.label(),
        Duration::from_millis(500),
    );
    logger.disable_file_logging();
    logger.writer.flush(Duration::from_secs(2));

    let contents = std::fs::read_to_string(path).unwrap();
    assert!(contents.contains("component=docx_viewer"));
    assert!(contents.contains("event=opening_stage"));
    assert!(contents.contains("phase=read_file"));
    assert!(contents.contains("duration=25ms"));
    assert!(contents.contains("open_id=7"));
    assert!(contents.contains("phase=first_page_ui_drawn"));
    assert!(!contents.contains("stale_event"));
}

#[test]
fn first_page_state_is_one_shot_and_does_not_require_diagnostics() {
    let mut state = FirstPageTrace {
        trace: OpenTrace::default(),
        texture_recorded: false,
        drawn: false,
    };
    assert!(!state.texture_recorded);
    assert!(!state.drawn);
    state.texture_ready();
    state.page_drawn();
    assert!(state.texture_recorded);
    assert!(state.drawn);
    state.texture_ready();
    state.page_drawn();
    assert!(state.texture_stages().recording.is_none());
}
