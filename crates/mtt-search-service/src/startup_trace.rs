//! Temporary, opt-in startup/shutdown diagnostics. No filesystem paths or names
//! from indexed records are accepted by the event API.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static OUTPUT: OnceLock<Option<Mutex<File>>> = OnceLock::new();
static QUERY_STATES: [AtomicU32; 26] = [const { AtomicU32::new(0) }; 26];

pub(crate) fn query_state(drive: char, code: u32, name: &'static str) {
    if !drive.is_ascii_uppercase() || code >= 32 {
        return;
    }
    let bit = 1 << code;
    if QUERY_STATES[(drive as u8 - b'A') as usize].fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        event(Some(drive), name, &[]);
    }
}

pub(crate) fn init() {
    OUTPUT.get_or_init(|| {
        let directory = crate::index_db::data_dir();
        if !directory.join("startup_trace.enabled").is_file() {
            return None;
        }
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(directory.join("startup_trace.log"))
            .ok()
            .map(Mutex::new)
    });
    event(None, "process_started", &[]);
}

pub(crate) fn event(drive: Option<char>, name: &'static str, fields: &[(&'static str, u64)]) {
    let Some(Some(output)) = OUTPUT.get() else {
        return;
    };
    if !crate::index_db::data_dir()
        .join("startup_trace.enabled")
        .is_file()
    {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut file = output.lock().unwrap_or_else(|error| error.into_inner());
    let mut line = format!(
        "{timestamp}\t{}\t{}\t{name}",
        std::process::id(),
        drive.unwrap_or('-')
    );
    for (key, value) in fields {
        line.push_str(&format!("\t{key}={value}"));
    }
    line.push('\n');
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
    let _ = file.sync_data();
}

pub(crate) fn error(drive: char, name: &'static str, message: &str) {
    // Extract only the numeric OS error; never persist the original message.
    let code = message
        .split("(os error ")
        .nth(1)
        .and_then(|tail| tail.split(')').next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    event(
        Some(drive),
        name,
        &[
            ("os_error", code),
            ("hmac_error", u64::from(message.contains("HMAC"))),
        ],
    );
}
