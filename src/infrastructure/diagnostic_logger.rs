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

static LOGGER: OnceLock<DiagnosticLogger> = OnceLock::new();

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

    fn write_record(&mut self, record: &Record<'_>) -> std::io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_record(&mut writer, record)?;
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

        fs::create_dir_all(log_dir).map_err(|err| {
            format!(
                "Failed to create diagnostic log directory '{}': {}",
                log_dir.display(),
                err
            )
        })?;
        truncate_if_oversized(&path).map_err(|err| {
            format!(
                "Failed to truncate oversized diagnostic log '{}': {}",
                path.display(),
                err
            )
        })?;

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)
            .map_err(|err| format!("Failed to open diagnostic log '{}': {}", path.display(), err))?;

        let mut diagnostic_file = DiagnosticFile::from_file(path.clone(), file).map_err(|err| {
            format!(
                "Failed to inspect diagnostic log '{}': {}",
                path.display(),
                err
            )
        })?;
        diagnostic_file.write_session_header(enabled_since).map_err(|err| {
            format!(
                "Failed to write diagnostic log header '{}': {}",
                path.display(),
                err
            )
        })?;

        {
            let mut state = self.state.lock();
            state.file = Some(diagnostic_file);
            state.enabled_since = Some(enabled_since);
        }

        log::set_max_level(self.base_level.max(LevelFilter::Info));
        Ok(path)
    }

    fn disable_file_logging(&self) {
        let mut state = self.state.lock();
        if let Some(mut file) = state.file.take() {
            let _ = file.write_session_footer();
            let _ = file.writer.flush();
        }
        state.enabled_since = None;
        log::set_max_level(self.base_level);
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
}

impl Log for DiagnosticLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        if self.console.enabled(metadata) {
            return true;
        }

        metadata.level() <= Level::Info && self.is_enabled()
    }

    fn log(&self, record: &Record<'_>) {
        if self.console.matches(record) {
            self.console.log(record);
        }

        if record.level() > Level::Info {
            return;
        }

        let mut state = self.state.lock();
        if let Some(file) = state.file.as_mut() {
            let _ = file.write_record(record);
        }
    }

    fn flush(&self) {
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
    fs::create_dir_all(&log_dir).map_err(|err| {
        format!(
            "Failed to create diagnostic log directory '{}': {}",
            log_dir.display(),
            err
        )
    })?;
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
    writeln!(writer)?;
    writeln!(writer, "===== Diagnostic Session Started =====")?;
    writeln!(writer, "timestamp_ms={}", unix_millis(SystemTime::now()))?;
    writeln!(writer, "enabled_since_epoch_s={}", unix_secs(enabled_since))?;
    writeln!(writer, "version={}", env!("CARGO_PKG_VERSION"))?;
    writeln!(writer, "os={} arch={}", std::env::consts::OS, std::env::consts::ARCH)?;
    writeln!(writer, "exe={}", display_path(std::env::current_exe().ok()))?;
    writeln!(writer, "cwd={}", display_path(std::env::current_dir().ok()))?;
    writeln!(writer, "=====================================")?;
    Ok(())
}

fn write_session_footer<W: Write>(writer: &mut W) -> std::io::Result<()> {
    writeln!(writer, "===== Diagnostic Session Ended =====")?;
    writeln!(writer, "timestamp_ms={}", unix_millis(SystemTime::now()))?;
    Ok(())
}

fn write_record<W: Write>(writer: &mut W, record: &Record<'_>) -> std::io::Result<()> {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    writeln!(
        writer,
        "[{}][{}][{}][{}] {}",
        unix_millis(SystemTime::now()),
        record.level(),
        thread_name,
        record.target(),
        record.args()
    )
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

fn display_path(path: Option<PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unavailable>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
    fn write_record_reapplies_size_cap_after_overflow() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        let prefix_len = (MAX_LOG_BYTES - 64) as usize;
        let prefix = vec![b'a'; prefix_len];
        let message = "b".repeat(256);

        fs::write(&path, &prefix).unwrap();

        let mut file = open_diagnostic_file(&path);
        let args = format_args!("{}", message);
        let record = Record::builder()
            .args(args)
            .level(Level::Info)
            .target("diagnostic_logger_test")
            .build();
        file.write_record(&record).unwrap();
        file.writer.flush().unwrap();

        let contents = fs::read(&path).unwrap();
        assert!(contents.len() as u64 <= MAX_LOG_BYTES);
        assert!(contents.ends_with(format!("{message}\n").as_bytes()));
    }
}
