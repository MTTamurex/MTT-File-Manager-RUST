use super::*;
use std::fs::{self, OpenOptions};
use std::path::Path;
use tempfile::tempdir;

fn make_console_logger() -> Logger {
    let mut builder = env_logger::Builder::new();
    builder.filter_level(LevelFilter::Off);
    builder.build()
}

fn make_diagnostic_filter(level: LevelFilter) -> Logger {
    let mut builder = env_logger::Builder::new();
    builder.filter_level(level);
    builder.build()
}

fn open_diagnostic_file(path: &Path) -> DiagnosticFile {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .unwrap();
    DiagnosticFile::from_file(path.to_path_buf(), file).unwrap()
}

#[test]
fn regular_logs_do_not_mirror_into_diagnostic_file() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = DiagnosticLogger::new(
        make_console_logger(),
        make_diagnostic_filter(LevelFilter::Info),
    );
    let args = format_args!("renderer=Glow elapsed_ms=42");
    let record = Record::builder()
        .args(args)
        .level(Level::Info)
        .target("diagnostic_logger_test")
        .build();

    logger.log(&record);
    logger.flush_file();

    assert!(!path.exists());
}

#[test]
fn info_logs_are_mirrored_only_while_diagnostic_mode_is_active() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = DiagnosticLogger::new(
        make_console_logger(),
        make_diagnostic_filter(LevelFilter::Info),
    );
    let args = format_args!("renderer=Glow elapsed_ms=42");
    let record = Record::builder()
        .args(args)
        .level(Level::Info)
        .target("mtt_file_manager::startup")
        .build();

    {
        let mut state = logger.state.lock();
        state.file = Some(open_diagnostic_file(&path));
        state.enabled_since = Some(SystemTime::now());
    }

    assert!(logger.enabled(record.metadata()));
    logger.log(&record);
    logger.flush_file();

    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("event=info_log"));
    assert!(contents.contains("renderer=Glow elapsed_ms=42"));
}

#[test]
fn info_logs_resume_after_diagnostic_logging_is_reenabled() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = DiagnosticLogger::new(
        make_console_logger(),
        make_diagnostic_filter(LevelFilter::Info),
    );

    let before_disable = Record::builder()
        .args(format_args!("phase=before_disable"))
        .level(Level::Info)
        .target("mtt_file_manager::startup")
        .build();
    {
        let mut state = logger.state.lock();
        state.file = Some(DiagnosticFile::open(path.clone(), SystemTime::now()).unwrap());
        state.enabled_since = Some(SystemTime::now());
    }
    logger.log(&before_disable);
    logger.flush_file();

    logger.disable_file_logging();
    assert!(!logger.is_enabled());

    let while_disabled = Record::builder()
        .args(format_args!("phase=while_disabled"))
        .level(Level::Info)
        .target("mtt_file_manager::startup")
        .build();
    assert!(!logger.enabled(while_disabled.metadata()));
    logger.log(&while_disabled);

    {
        let mut state = logger.state.lock();
        let enabled_since = SystemTime::now();
        state.file = Some(DiagnosticFile::open(path.clone(), enabled_since).unwrap());
        state.enabled_since = Some(enabled_since);
    }

    let after_reenable = Record::builder()
        .args(format_args!("phase=after_reenable"))
        .level(Level::Info)
        .target("mtt_file_manager::startup")
        .build();
    assert!(logger.enabled(after_reenable.metadata()));
    logger.log(&after_reenable);
    logger.flush_file();

    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("phase=before_disable"));
    assert!(contents.contains("phase=after_reenable"));
    assert!(!contents.contains("phase=while_disabled"));
    assert_eq!(contents.matches("event=info_log").count(), 2);
    assert_eq!(contents.matches("event=session_started").count(), 2);
}

#[test]
fn warn_logs_are_mirrored_with_sensitive_data_redacted() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = DiagnosticLogger::new(
        make_console_logger(),
        make_diagnostic_filter(LevelFilter::Warn),
    );
    let args = format_args!(
        "[GLOBAL-SEARCH] Error for 'sample query': Failed to read C:\\Users\\user\\Documents\\Project\\sample.xlsx"
    );
    let record = Record::builder()
        .args(args)
        .level(Level::Warn)
        .target("mtt_file_manager::global_search")
        .build();

    {
        let mut state = logger.state.lock();
        state.file = Some(open_diagnostic_file(&path));
        state.enabled_since = Some(SystemTime::now());
    }

    logger.log(&record);
    logger.flush_file();

    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("component=runtime_log"));
    assert!(contents.contains("event=warn_log"));
    assert!(contents.contains("target=\"mtt_file_manager::global_search\""));
    assert!(contents
        .contains("message=\"[GLOBAL-SEARCH] Error for <redacted>: Failed to read <path>\""));
    assert!(!contents.contains("sample query"));
    assert!(!contents.contains("C:\\Users\\user"));
    assert!(!contents.contains("Project"));
    assert!(!contents.contains("sample.xlsx"));
}

#[test]
fn memory_trim_events_are_flushed_while_logging_remains_enabled() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(LOG_FILE_NAME);
    let logger = DiagnosticLogger::new(
        make_console_logger(),
        make_diagnostic_filter(LevelFilter::Info),
    );
    {
        let mut state = logger.state.lock();
        state.file = Some(open_diagnostic_file(&path));
        state.enabled_since = Some(SystemTime::now());
    }

    logger.write_event(
        Level::Info,
        "thumbnail_worker",
        "cache_miss",
        &[field_u64("count", 1)],
    );
    assert!(fs::read(&path).unwrap().is_empty());

    logger.write_event(
        Level::Info,
        "memory_trim",
        "background_status",
        &[field_u64("working_set_bytes", 123)],
    );

    let contents = fs::read_to_string(&path).unwrap();
    assert!(contents.contains("component=thumbnail_worker"));
    assert!(contents.contains("component=memory_trim"));
    assert!(contents.contains("working_set_bytes=123"));
    assert!(logger.is_enabled());
}
