use super::DiagnosticField;
use log::Level;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const TAIL_LOG_BYTES: u64 = 5 * 1024 * 1024;

pub(super) struct DiagnosticFile {
    path: PathBuf,
    writer: BufWriter<File>,
    bytes_written: u64,
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
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.inner.write_all(buf)?;
        self.bytes_written = self.bytes_written.saturating_add(buf.len() as u64);
        Ok(())
    }
}

impl DiagnosticFile {
    pub(super) fn from_file(path: PathBuf, file: File) -> io::Result<Self> {
        let bytes_written = file.metadata()?.len();
        Ok(Self {
            path,
            writer: BufWriter::new(file),
            bytes_written,
        })
    }

    pub(super) fn open(path: PathBuf, enabled_since: SystemTime) -> io::Result<Self> {
        truncate_if_oversized(&path)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        let mut diagnostic_file = Self::from_file(path, file)?;
        diagnostic_file.write_session_header(enabled_since)?;
        diagnostic_file.flush()?;
        Ok(diagnostic_file)
    }

    pub(super) fn write_event(
        &mut self,
        level: Level,
        component: &'static str,
        event_code: &'static str,
        fields: &[DiagnosticField],
    ) -> io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_event(&mut writer, level, component, event_code, fields)?;
            writer.bytes_written()
        };
        self.finish_write(written)
    }

    pub(super) fn write_session_footer(&mut self) -> io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_session_footer(&mut writer)?;
            writer.bytes_written()
        };
        self.finish_write(written)
    }

    pub(super) fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }

    fn write_session_header(&mut self, enabled_since: SystemTime) -> io::Result<()> {
        let written = {
            let mut writer = CountingWriter::new(&mut self.writer);
            write_session_header(&mut writer, enabled_since)?;
            writer.bytes_written()
        };
        self.finish_write(written)
    }

    fn finish_write(&mut self, written: u64) -> io::Result<()> {
        self.bytes_written = self.bytes_written.saturating_add(written);
        if self.bytes_written <= MAX_LOG_BYTES {
            return Ok(());
        }

        self.writer.flush()?;
        truncate_if_oversized(&self.path)?;
        self.reopen()
    }

    fn reopen(&mut self) -> io::Result<()> {
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

pub(super) fn truncate_if_oversized(path: &Path) -> io::Result<()> {
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

fn write_session_header<W: Write>(writer: &mut W, enabled_since: SystemTime) -> io::Result<()> {
    let fields = [
        DiagnosticField::Label("schema", super::DIAGNOSTIC_SCHEMA_VERSION),
        DiagnosticField::U64("enabled_since_epoch_s", unix_secs(enabled_since)),
        DiagnosticField::Label("version", env!("CARGO_PKG_VERSION")),
        DiagnosticField::Label("os", std::env::consts::OS),
        DiagnosticField::Label("arch", std::env::consts::ARCH),
    ];
    write_event(
        writer,
        Level::Info,
        "diagnostic_mode",
        "session_started",
        &fields,
    )
}

fn write_session_footer<W: Write>(writer: &mut W) -> io::Result<()> {
    write_event(writer, Level::Info, "diagnostic_mode", "session_ended", &[])
}

fn write_event<W: Write>(
    writer: &mut W,
    level: Level,
    component: &'static str,
    event_code: &'static str,
    fields: &[DiagnosticField],
) -> io::Result<()> {
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
            DiagnosticField::Label(key, value) => write!(
                writer,
                " {}={}",
                key,
                super::privacy::sanitize_log_message(value)
            )?,
            DiagnosticField::Text(key, value) => write!(
                writer,
                " {}=\"{}\"",
                key,
                escape_text_field(&super::privacy::sanitize_log_message(value))
            )?,
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

pub(super) fn runtime_event_code_for(level: Level) -> &'static str {
    runtime_event_code(level)
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
    fn session_header_excludes_runtime_paths() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("diagnostic.log");
        let mut file = open_diagnostic_file(&path);

        file.write_session_header(SystemTime::UNIX_EPOCH).unwrap();
        file.flush().unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("schema=privacy_safe_v3"));
        assert!(contents.contains("component=diagnostic_mode"));
        assert!(!contents.contains("cwd="));
        assert!(!contents.contains("exe="));
        assert!(!contents.contains("C:\\"));
    }

    #[test]
    fn diagnostic_events_serialize_only_safe_scalar_fields() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("diagnostic.log");
        let mut file = open_diagnostic_file(&path);
        let fields = [
            DiagnosticField::Bool("enabled", true),
            DiagnosticField::I64("error_code", -5),
            DiagnosticField::U64("attempt", 3),
            DiagnosticField::DurationMs("elapsed", 1250),
            DiagnosticField::Label("result", "timeout"),
            DiagnosticField::Text(
                "path",
                r"C:\Users\user\Documents\Project Name\sample.txt".to_string(),
            ),
        ];

        file.write_event(Level::Warn, "thumbnail_worker", "metadata_timeout", &fields)
            .unwrap();
        file.flush().unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("level=warn"));
        assert!(contents.contains("component=thumbnail_worker"));
        assert!(contents.contains("event=metadata_timeout"));
        assert!(contents.contains("enabled=true"));
        assert!(contents.contains("error_code=-5"));
        assert!(contents.contains("attempt=3"));
        assert!(contents.contains("elapsed=1250ms"));
        assert!(contents.contains("result=timeout"));
        assert!(contents.contains("path=\"<path>\""));
        assert!(!contents.contains("Users"));
        assert!(!contents.contains("Project Name"));
        assert!(!contents.contains("sample.txt"));
    }

    #[test]
    fn write_event_reapplies_size_cap_after_overflow() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("diagnostic.log");
        let prefix_len = (MAX_LOG_BYTES - 64) as usize;
        let prefix = vec![b'a'; prefix_len];

        fs::write(&path, &prefix).unwrap();

        let mut file = open_diagnostic_file(&path);
        let fields = [DiagnosticField::Label("result", "overflow")];
        file.write_event(
            Level::Info,
            "diagnostic_mode",
            "size_cap_reapplied",
            &fields,
        )
        .unwrap();
        file.flush().unwrap();

        let contents = fs::read(&path).unwrap();
        assert!(contents.len() as u64 <= MAX_LOG_BYTES);
        assert!(String::from_utf8_lossy(&contents).contains("event=size_cap_reapplied"));
    }
}
