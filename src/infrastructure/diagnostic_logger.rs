use env_logger::Logger;
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use parking_lot::Mutex;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DIAGNOSTIC_MODE_KEY: &str = "diagnostic_mode";
pub const DIAGNOSTIC_MODE_ENABLED_AT_KEY: &str = "diagnostic_mode_enabled_at";
pub const AUTO_DISABLE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

const LOG_FILE_NAME: &str = "diagnostic.log";
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const TAIL_LOG_BYTES: u64 = 5 * 1024 * 1024;
const DIAGNOSTIC_SCHEMA_VERSION: &str = "privacy_safe_v2";
const MAX_TEXT_FIELD_LEN: usize = 240;

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

struct DiagnosticFile {
    path: PathBuf,
    writer: BufWriter<File>,
    bytes_written: u64,
}

#[derive(Default)]
struct DiagnosticState {
    file: Option<DiagnosticFile>,
    enabled_since: Option<SystemTime>,
}

pub struct DiagnosticLogger {
    console: Logger,
    base_level: LevelFilter,
    state: Mutex<DiagnosticState>,
}

struct CountingWriter<W> {
    inner: W,
    bytes_written: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            bytes_written: 0,
        }
    }

    fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.inner.write_all(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(buf.len() as u64);
        Ok(())
    }
}

impl DiagnosticFile {
    fn from_file(path: PathBuf, file: File) -> std::io::Result<Self> {
        let bytes_written = file.metadata()?.len();
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            bytes_written,
        })
    }

    fn write_session_header(&mut self, enabled_since: SystemTime) -> std::io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_session_header(&mut writer, enabled_since)?;
            writer.bytes_written()
        };
        self.finish_write(written)?;
        self.writer.flush()
    }

    fn write_session_footer(&mut self) -> std::io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_session_footer(&mut writer)?;
            writer.bytes_written()
        };
        self.finish_write(written)
    }

    fn write_event(
        &mut self,
        level: Level,
        component: &'static str,
        event_code: &'static str,
        fields: &[DiagnosticField],
    ) -> std::io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_event(&mut writer, level, component, event_code, fields)?;
            writer.bytes_written()
        };
        self.finish_write(written)
    }

    fn finish_write(&mut self, written: u64) -> std::io::Result<()> {
        self.bytes_written = self.bytes_written.saturating_add(written);
        if self.bytes_written <= MAX_LOG_BYTES {
            return Ok(());
        }

        self.writer.flush()?;
        truncate_if_oversized(&self.path)?;
        self.reopen()
    }

    fn reopen(&mut self) -> std::io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&self.path)?;
        self.bytes_written = file.metadata()?.len();
        self.writer = BufWriter::new(file);
        Ok(())
    }

}

impl DiagnosticLogger {
    fn new(console: Logger) -> Self {
        Self {
            base_level: console.filter(),
            console,
            state: Mutex::new(DiagnosticState::default()),
        }
    }

    fn enable_file_logging_with_since(&self, enabled_since: SystemTime) -> Result<PathBuf, String> {
        let path = log_file_path();
        let log_dir = path
            .parent()
            .ok_or_else(|| "Diagnostic log path has no parent directory".to_string())?;

        fs::create_dir_all(log_dir)
            .map_err(|err| format!("Failed to create diagnostic log directory: {}", err))?;
        truncate_if_oversized(&path)
            .map_err(|err| format!("Failed to truncate oversized diagnostic log: {}", err))?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .map_err(|err| format!("Failed to open diagnostic log: {}", err))?;

        let mut diagnostic_file = DiagnosticFile::from_file(path.clone(), file)
            .map_err(|err| format!("Failed to inspect diagnostic log: {}", err))?;
        diagnostic_file
            .write_session_header(enabled_since)
            .map_err(|err| format!("Failed to write diagnostic log header: {}", err))?;

        {
            let mut state = self.state.lock();
            state.file = Some(diagnostic_file);
            state.enabled_since = Some(enabled_since);
        }

        Ok(path)
    }

    fn disable_file_logging(&self) {
        let mut state = self.state.lock();
        if let Some(mut file) = state.file.take() {
            let _ = file.write_session_footer();
            let _ = file.writer.flush();
        }
        state.enabled_since = None;
    }

    fn is_enabled(&self) -> bool {
        self.state.lock().file.is_some()
    }

    fn enabled_since(&self) -> Option<SystemTime> {
        self.state.lock().enabled_since
    }

    fn flush_file(&self) {
        if let Some(file) = self.state.lock().file.as_mut() {
            let _ = file.writer.flush();
        }
    }

    fn write_event(
        &self,
        level: Level,
        component: &'static str,
        event_code: &'static str,
        fields: &[DiagnosticField],
    ) {
        let mut state = self.state.lock();
        if let Some(file) = state.file.as_mut() {
            let _ = file.write_event(level, component, event_code, fields);
        }
    }

    fn write_runtime_record(&self, record: &Record<'_>) {
        let message = sanitize_log_message(&record.args().to_string());
        if message.is_empty() {
            return;
        }

        let fields = [
            DiagnosticField::Text("target", normalize_text(record.target())),
            DiagnosticField::Text("message", message),
        ];

        self.write_event(
            record.level(),
            "runtime_log",
            runtime_event_code(record.level()),
            &fields,
        );
    }
}

impl Log for DiagnosticLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.console.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if self.console.matches(record) {
            self.console.log(record);
        }

        if matches!(record.level(), Level::Warn | Level::Error) {
            self.write_runtime_record(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
        self.flush_file();
    }
}

pub fn init(console: Logger) -> Result<(), SetLoggerError> {
    let logger = LOGGER.get_or_init(|| DiagnosticLogger::new(console));
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
    logger.enable_file_logging_with_since(enabled_since)
}

pub fn disable_file_logging() {
    if let Some(logger) = LOGGER.get() {
        logger.disable_file_logging();
    }
}

pub fn is_enabled() -> bool {
    LOGGER.get().map(|logger| logger.is_enabled()).unwrap_or(false)
}

pub fn enabled_since() -> Option<SystemTime> {
    LOGGER.get().and_then(|logger| logger.enabled_since())
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
    crate::infrastructure::windows::open_with_shell(&log_dir)
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

fn truncate_if_oversized(path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };

    if metadata.len() <= MAX_LOG_BYTES {
        return Ok(());
    }

    let keep = TAIL_LOG_BYTES.min(metadata.len());
    let mut input = File::open(path)?;
    input.seek(SeekFrom::End(-(keep as i64)))?;

    let mut tail = vec![0_u8; keep as usize];
    input.read_exact(&mut tail)?;
    drop(input);

    let mut output = File::create(path)?;
    output.write_all(&tail)?;
    output.flush()?;
    Ok(())
}

fn write_session_header<W: Write>(writer: &mut W, enabled_since: SystemTime) -> std::io::Result<()> {
    let fields = [
        field_label("schema", DIAGNOSTIC_SCHEMA_VERSION),
        field_u64("enabled_since_epoch_s", unix_secs(enabled_since)),
        field_label("version", env!("CARGO_PKG_VERSION")),
        field_label("os", std::env::consts::OS),
        field_label("arch", std::env::consts::ARCH),
    ];
    write_event(
        writer,
        Level::Info,
        "diagnostic_mode",
        "session_started",
        &fields,
    )
}

fn write_session_footer<W: Write>(writer: &mut W) -> std::io::Result<()> {
    write_event(writer, Level::Info, "diagnostic_mode", "session_ended", &[])
}

fn write_event<W: Write>(
    writer: &mut W,
    level: Level,
    component: &'static str,
    event_code: &'static str,
    fields: &[DiagnosticField],
) -> std::io::Result<()> {
    write!(
        writer,
        "ts_ms={} level={} component={} event={}",
        unix_millis(SystemTime::now()),
        level_token(level),
        component,
        event_code
    )?;

    for field in fields {
        match field {
            DiagnosticField::Bool(key, value) => write!(writer, " {}={}", key, value)?,
            DiagnosticField::I64(key, value) => write!(writer, " {}={}", key, value)?,
            DiagnosticField::U64(key, value) => write!(writer, " {}={}", key, value)?,
            DiagnosticField::DurationMs(key, value) => write!(writer, " {}={}ms", key, value)?,
            DiagnosticField::Label(key, value) => write!(writer, " {}={}", key, value)?,
            DiagnosticField::Text(key, value) => {
                write!(writer, " {}=\"{}\"", key, escape_text_field(value))?
            }
        }
    }

    writeln!(writer)
}

fn level_token(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    }
}

fn runtime_event_code(level: Level) -> &'static str {
    match level {
        Level::Error => "error_log",
        Level::Warn => "warn_log",
        Level::Info => "info_log",
        Level::Debug => "debug_log",
        Level::Trace => "trace_log",
    }
}

fn normalize_text(input: &str) -> String {
    truncate_text(&input.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn sanitize_log_message(input: &str) -> String {
    let redacted_quotes = redact_quoted_segments(input);
    let normalized = redacted_quotes
        .split_whitespace()
        .map(sanitize_message_token)
        .collect::<Vec<_>>()
        .join(" ");
    truncate_text(&normalized)
}

fn redact_quoted_segments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\'' || ch == '"' {
            let quote = ch;
            i += 1;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            if !out.ends_with("<redacted>") {
                out.push_str("<redacted>");
            }
            if i < chars.len() {
                i += 1;
            }
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}

fn sanitize_message_token(token: &str) -> String {
    let (prefix, core, suffix) = split_wrapping_punctuation(token);
    if core.is_empty() {
        return token.to_string();
    }

    if let Some((key, value)) = core.split_once('=') {
        if is_sensitive_key(key) || looks_sensitive_fragment(value) {
            return format!("{prefix}{key}=<redacted>{suffix}");
        }
        return format!("{prefix}{key}={}{suffix}", sanitize_fragment(value));
    }

    format!("{prefix}{}{suffix}", sanitize_fragment(core))
}

fn sanitize_fragment(fragment: &str) -> String {
    if looks_sensitive_fragment(fragment) {
        "<redacted>".to_string()
    } else {
        fragment.to_string()
    }
}

fn split_wrapping_punctuation(token: &str) -> (&str, &str, &str) {
    let trim_chars: &[char] = &['(', ')', '[', ']', '{', '}', ',', ';'];
    let core = token.trim_matches(trim_chars);
    let start = token.find(core).unwrap_or(0);
    let end = start.saturating_add(core.len());
    (&token[..start], core, &token[end..])
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("path")
        || lower.contains("file")
        || lower.contains("folder")
        || lower.contains("query")
        || lower.contains("search")
        || lower.contains("name")
}

fn looks_sensitive_fragment(fragment: &str) -> bool {
    let trimmed = fragment.trim_matches(|c: char| {
        matches!(c, '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';')
    });

    if trimmed.is_empty() || trimmed == "<redacted>" {
        return false;
    }

    let bytes = trimmed.as_bytes();
    if trimmed.starts_with("\\\\")
        || trimmed.starts_with("//")
        || trimmed.starts_with("\\\\?\\")
        || trimmed.contains('\\')
        || trimmed.contains('/')
        || (bytes.len() >= 3
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
            && bytes[0].is_ascii_alphabetic())
    {
        return true;
    }

    looks_like_filename(trimmed)
}

fn looks_like_filename(fragment: &str) -> bool {
    let trimmed = fragment.trim_matches('.');
    if trimmed.contains("::") || !trimmed.contains('.') {
        return false;
    }

    let Some((base, ext)) = trimmed.rsplit_once('.') else {
        return false;
    };

    !base.is_empty()
        && base.chars().any(|ch| ch.is_ascii_alphabetic())
        && !ext.is_empty()
        && ext.len() <= 8
        && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
        && ext.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn truncate_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len().min(MAX_TEXT_FIELD_LEN));
    for ch in input.chars().take(MAX_TEXT_FIELD_LEN) {
        out.push(ch);
    }
    out
}

fn escape_text_field(input: &str) -> String {
    input.replace('\\', r"\\").replace('"', r#"\""#)
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_millis(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_console_logger() -> Logger {
        let mut builder = env_logger::Builder::new();
        builder.filter_level(LevelFilter::Off);
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
        let logger = DiagnosticLogger::new(make_console_logger());
        let args = format_args!("path=C:\\Users\\Alice\\secret.txt");
        let record = Record::builder()
            .args(args)
            .level(Level::Info)
            .target("diagnostic_logger_test")
            .build();

        {
            let mut state = logger.state.lock();
            state.file = Some(open_diagnostic_file(&path));
            state.enabled_since = Some(SystemTime::now());
        }

        logger.log(&record);
        logger.flush_file();

        assert!(fs::read(&path).unwrap().is_empty());
    }

    #[test]
    fn warn_logs_are_mirrored_with_sensitive_data_redacted() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        let logger = DiagnosticLogger::new(make_console_logger());
        let args = format_args!(
            "[GLOBAL-SEARCH] Error for 'budget 2026': Failed to read C:\\Users\\Alice\\Secrets\\taxes.xlsx"
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
        assert!(contents.contains("message=\"[GLOBAL-SEARCH] Error for <redacted>: Failed to read <redacted>\""));
        assert!(!contents.contains("budget 2026"));
        assert!(!contents.contains("C:\\Users\\Alice"));
        assert!(!contents.contains("taxes.xlsx"));
    }

    #[test]
    fn session_header_excludes_runtime_paths() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        let mut file = open_diagnostic_file(&path);

        file.write_session_header(SystemTime::UNIX_EPOCH).unwrap();
        file.writer.flush().unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("schema=privacy_safe_v2"));
        assert!(contents.contains("component=diagnostic_mode"));
        assert!(!contents.contains("cwd="));
        assert!(!contents.contains("exe="));
        assert!(!contents.contains("C:\\"));
    }

    #[test]
    fn diagnostic_events_serialize_only_safe_scalar_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        let mut file = open_diagnostic_file(&path);
        let fields = [
            field_bool("enabled", true),
            field_i64("error_code", -5),
            field_u64("attempt", 3),
            field_duration_ms("elapsed", Duration::from_millis(1250)),
            field_label("result", "timeout"),
        ];

        file.write_event(Level::Warn, "thumbnail_worker", "metadata_timeout", &fields)
            .unwrap();
        file.writer.flush().unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("level=warn"));
        assert!(contents.contains("component=thumbnail_worker"));
        assert!(contents.contains("event=metadata_timeout"));
        assert!(contents.contains("enabled=true"));
        assert!(contents.contains("error_code=-5"));
        assert!(contents.contains("attempt=3"));
        assert!(contents.contains("elapsed=1250ms"));
        assert!(contents.contains("result=timeout"));
    }

    #[test]
    fn write_event_reapplies_size_cap_after_overflow() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        let prefix_len = (MAX_LOG_BYTES - 64) as usize;
        let prefix = vec![b'a'; prefix_len];

        fs::write(&path, &prefix).unwrap();

        let mut file = open_diagnostic_file(&path);
        let fields = [field_label("result", "overflow")];
        file.write_event(Level::Info, "diagnostic_mode", "size_cap_reapplied", &fields)
            .unwrap();
        file.writer.flush().unwrap();

        let contents = fs::read(&path).unwrap();
        assert!(contents.len() as u64 <= MAX_LOG_BYTES);
        assert!(String::from_utf8_lossy(&contents).contains("event=size_cap_reapplied"));
    }
}
