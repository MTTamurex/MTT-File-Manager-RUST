pub(crate) mod frame_trace;
pub(crate) mod navigation_trace;
mod slow_scope;
mod writer;
pub(crate) use slow_scope::SlowScope;
mod privacy;
mod storage;

use env_logger::Logger;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use storage::DiagnosticFile;

pub const DIAGNOSTIC_MODE_KEY: &str = "diagnostic_mode";
pub const DIAGNOSTIC_MODE_ENABLED_AT_KEY: &str = "diagnostic_mode_enabled_at";
pub const AUTO_DISABLE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

const LOG_FILE_NAME: &str = "diagnostic.log";
const DIAGNOSTIC_SCHEMA_VERSION: &str = "privacy_safe_v3";

static LOGGER: OnceLock<DiagnosticLogger> = OnceLock::new();

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiagnosticField {
    Bool(&'static str, bool),
    I64(&'static str, i64),
    U64(&'static str, u64),
    DurationMs(&'static str, u64),
    Label(&'static str, &'static str),
    Text(&'static str, String),
}

pub struct DiagnosticLogger {
    console: Logger,
    diagnostic_filter: Logger,
    base_level: LevelFilter,
    writer: writer::Writer,
    enabled_since: parking_lot::Mutex<Option<SystemTime>>,
    // Odd values identify an enabled session; reads never wait for file I/O.
    capture_session: AtomicU64,
}

impl DiagnosticLogger {
    fn new(console: Logger, diagnostic_filter: Logger) -> Self {
        Self {
            base_level: console.filter(),
            console,
            diagnostic_filter,
            writer: writer::Writer::new(),
            enabled_since: parking_lot::Mutex::new(None),
            capture_session: AtomicU64::new(0),
        }
    }

    fn enable_file_logging_with_since(&self, enabled_since: SystemTime) -> Result<PathBuf, String> {
        // Serialize lifecycle transitions only; the writer never takes this lock.
        let mut state = self.enabled_since.lock();
        let path = log_file_path();
        let session = self.capture_session.load(Ordering::Acquire).wrapping_add(2) | 1;
        self.writer.open(session, path.clone(), enabled_since)?;
        *state = Some(enabled_since);
        self.capture_session.store(session, Ordering::Release);
        Ok(path)
    }

    fn disable_file_logging(&self) {
        let mut state = self.enabled_since.lock();
        let session = self.capture_session.fetch_and(!1, Ordering::AcqRel);
        if session & 1 != 0 {
            self.writer.close(session);
        }
        *state = None;
    }

    fn is_enabled(&self) -> bool {
        self.capture_session().is_some()
    }

    fn capture_session(&self) -> Option<u64> {
        let value = self.capture_session.load(Ordering::Acquire);
        (value & 1 != 0).then_some(value)
    }

    fn enabled_since(&self) -> Option<SystemTime> {
        *self.enabled_since.lock()
    }

    fn flush_file(&self) {
        self.writer.flush(Duration::from_millis(250));
    }

    fn write_event(
        &self,
        level: Level,
        component: &'static str,
        event_code: &'static str,
        fields: &[DiagnosticField],
    ) {
        if let Some(session) = self.capture_session() {
            self.writer.events(
                session,
                vec![writer::Event::new(
                    level,
                    component,
                    event_code,
                    fields.to_vec(),
                )],
            );
        }
    }

    fn write_runtime_record(&self, record: &Record<'_>) {
        let message = privacy::sanitize_log_message(&record.args().to_string());
        if message.is_empty() {
            return;
        }

        let fields = [
            DiagnosticField::Text("target", privacy::sanitize_log_message(record.target())),
            DiagnosticField::Text("message", message),
        ];

        self.write_event(
            record.level(),
            "runtime_log",
            storage::runtime_event_code_for(record.level()),
            &fields,
        );
    }

    fn diagnostic_max_level(&self) -> LevelFilter {
        self.base_level.max(self.diagnostic_filter.filter())
    }
}

impl Log for DiagnosticLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.console.enabled(metadata)
            || (self.is_enabled() && self.diagnostic_filter.enabled(metadata))
    }

    fn log(&self, record: &Record<'_>) {
        if self.console.matches(record) {
            self.console.log(record);
        }

        if self.is_enabled() && self.diagnostic_filter.matches(record) {
            self.write_runtime_record(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
        self.flush_file();
    }
}

pub fn init(console: Logger, diagnostic_filter: Logger) -> Result<(), SetLoggerError> {
    let logger = LOGGER.get_or_init(|| DiagnosticLogger::new(console, diagnostic_filter));
    log::set_logger(logger)?;
    log::set_max_level(logger.base_level);
    Ok(())
}

pub fn enable_file_logging() -> Result<PathBuf, String> {
    enable_file_logging_with_since(SystemTime::now())
}

pub fn enable_file_logging_with_since(enabled_since: SystemTime) -> Result<PathBuf, String> {
    let logger = LOGGER
        .get()
        .ok_or_else(|| "Diagnostic logger is not initialized".to_string())?;
    let path = logger.enable_file_logging_with_since(enabled_since)?;
    log::set_max_level(logger.diagnostic_max_level());
    Ok(path)
}

pub fn disable_file_logging() {
    if let Some(logger) = LOGGER.get() {
        logger.disable_file_logging();
        log::set_max_level(logger.base_level);
    }
}

pub fn is_enabled() -> bool {
    LOGGER
        .get()
        .map(|logger| logger.is_enabled())
        .unwrap_or(false)
}

/// Non-blocking session token for performance instrumentation only.
pub(crate) fn capture_session() -> Option<u64> {
    LOGGER.get()?.capture_session()
}

pub fn enabled_since() -> Option<SystemTime> {
    LOGGER.get().and_then(|logger| logger.enabled_since())
}

/// Includes asynchronous open/write failures without consulting the filesystem.
pub fn writer_failures() -> u64 {
    LOGGER.get().map_or(0, |logger| logger.writer.failures())
}

pub fn take_output_failure() -> bool {
    LOGGER
        .get()
        .is_some_and(|logger| logger.writer.take_output_failure())
}

pub fn flush() {
    if let Some(logger) = LOGGER.get() {
        logger.flush_file();
    }
}

pub fn field_bool(key: &'static str, value: bool) -> DiagnosticField {
    DiagnosticField::Bool(key, value)
}

pub fn field_i64(key: &'static str, value: i64) -> DiagnosticField {
    DiagnosticField::I64(key, value)
}

pub fn field_u64(key: &'static str, value: u64) -> DiagnosticField {
    DiagnosticField::U64(key, value)
}

pub fn field_duration_ms(key: &'static str, value: Duration) -> DiagnosticField {
    DiagnosticField::DurationMs(key, value.as_millis().try_into().unwrap_or(u64::MAX))
}

pub fn field_label(key: &'static str, value: &'static str) -> DiagnosticField {
    DiagnosticField::Label(key, value)
}

pub fn diag_info(component: &'static str, event_code: &'static str, fields: &[DiagnosticField]) {
    diag_event(Level::Info, component, event_code, fields);
}

pub fn diag_warn(component: &'static str, event_code: &'static str, fields: &[DiagnosticField]) {
    diag_event(Level::Warn, component, event_code, fields);
}

pub fn diag_error(component: &'static str, event_code: &'static str, fields: &[DiagnosticField]) {
    diag_event(Level::Error, component, event_code, fields);
}

fn diag_event(
    level: Level,
    component: &'static str,
    event_code: &'static str,
    fields: &[DiagnosticField],
) {
    if let Some(logger) = LOGGER.get() {
        logger.write_event(level, component, event_code, fields);
    }
}

pub fn log_directory_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("MTT-File-Manager")
        .join("logs")
}

pub fn log_file_path() -> PathBuf {
    log_directory_path().join(LOG_FILE_NAME)
}

pub fn open_log_folder() -> Result<PathBuf, String> {
    let log_dir = log_directory_path();
    fs::create_dir_all(&log_dir)
        .map_err(|err| format!("Failed to create diagnostic log directory: {}", err))?;
    crate::infrastructure::windows::open_with_shell(&log_dir, None)
        .map_err(|err| format!("Failed to open diagnostic log directory: {}", err))?;
    Ok(log_dir)
}

pub fn parse_enabled_at_preference(raw: Option<&str>) -> Option<SystemTime> {
    let secs = raw?.trim().parse::<u64>().ok()?;
    UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}

pub fn format_enabled_at_preference(enabled_since: Option<SystemTime>) -> Option<String> {
    let enabled_since = enabled_since?;
    let secs = enabled_since.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(secs.to_string())
}

pub fn is_preference_expired(enabled_since: Option<SystemTime>, now: SystemTime) -> bool {
    let Some(enabled_since) = enabled_since else {
        return false;
    };

    now.duration_since(enabled_since).unwrap_or_default() >= AUTO_DISABLE_AFTER
}

#[cfg(test)]
mod tests;
