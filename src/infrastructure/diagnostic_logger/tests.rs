use super::*;
use std::fs;
use tempfile::tempdir;

fn logger() -> DiagnosticLogger {
    let mut console = env_logger::Builder::new();
    console.filter_level(LevelFilter::Off);
    let mut diagnostic = env_logger::Builder::new();
    diagnostic.filter_level(LevelFilter::Info);
    DiagnosticLogger::new(console.build(), diagnostic.build())
}

fn activate(logger: &DiagnosticLogger, path: PathBuf, session: u64) {
    let since = SystemTime::now();
    logger.writer.open(session, path, since).unwrap();
    *logger.enabled_since.lock() = Some(since);
    logger.capture_session.store(session, Ordering::Release);
}

fn flush(logger: &DiagnosticLogger) {
    logger.writer.flush(Duration::from_secs(2));
}

#[test]
fn capture_and_enabled_status_do_not_consult_writer() {
    let logger = logger();
    assert_eq!(logger.capture_session(), None);
    logger.capture_session.store(1, Ordering::Release);
    assert!(logger.is_enabled());
    assert_eq!(logger.capture_session(), Some(1));
    logger.disable_file_logging();
    assert!(!logger.is_enabled());
}

#[test]
fn regular_logs_do_not_mirror_into_diagnostic_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = logger();
    logger.log(
        &Record::builder()
            .args(format_args!("elapsed_ms=42"))
            .level(Level::Info)
            .target("test")
            .build(),
    );
    flush(&logger);
    assert!(!path.exists());
}

#[test]
fn logs_resume_after_reenable_and_stale_batches_cannot_cross_sessions() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = logger();
    activate(&logger, path.clone(), 1);
    logger.write_event(Level::Info, "test", "before_disable", &[]);
    logger.disable_file_logging();
    logger.write_event(Level::Info, "test", "while_disabled", &[]);
    activate(&logger, path.clone(), 3);
    logger.writer.events(
        1,
        vec![writer::Event::new(Level::Info, "test", "stale", vec![])],
    );
    logger.write_event(Level::Info, "test", "after_reenable", &[]);
    logger.disable_file_logging();
    flush(&logger);
    let contents = fs::read_to_string(path).unwrap();
    assert!(contents.contains("event=before_disable"));
    assert!(contents.contains("event=after_reenable"));
    assert!(!contents.contains("event=while_disabled"));
    assert!(!contents.contains("event=stale"));
    assert_eq!(contents.matches("event=session_started").count(), 2);
    assert_eq!(contents.matches("event=session_ended").count(), 2);
}

#[test]
fn warn_logs_are_mirrored_with_sensitive_data_redacted() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = logger();
    activate(&logger, path.clone(), 1);
    let record = Record::builder().args(format_args!(
        "[GLOBAL-SEARCH] Error for 'sample query': Failed to read C:\\Users\\user\\Documents\\Project\\sample.xlsx"
    )).level(Level::Warn).target("mtt_file_manager::global_search").build();
    assert!(logger.enabled(record.metadata()));
    logger.log(&record);
    flush(&logger);
    let contents = fs::read_to_string(path).unwrap();
    assert!(contents.contains("event=warn_log"));
    assert!(contents.contains("target=\"mtt_file_manager::global_search\""));
    assert!(contents.contains("Error for <redacted>: Failed to read <path>"));
    for sensitive in ["sample query", "C:\\Users\\user", "Project", "sample.xlsx"] {
        assert!(!contents.contains(sensitive));
    }
}

#[test]
fn structured_metrics_survive_privacy_filter_and_include_capture_time() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = logger();
    activate(&logger, path.clone(), 1);
    logger.write_event(
        Level::Info,
        "frame_trace",
        "context",
        &[
            field_u64("folder_previews_loading", 12),
            field_duration_ms("duration", Duration::from_secs(3)),
        ],
    );
    logger.disable_file_logging();
    flush(&logger);
    let contents = fs::read_to_string(path).unwrap();
    assert!(contents.contains("folder_previews_loading=12"));
    assert!(contents.contains("duration=3000ms"));
    assert!(contents.contains("captured_at_epoch_ms="));
}
