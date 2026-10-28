//! The restart record is separate from debounced, process-local preferences.
use crate::{
    infrastructure::app_state_db::{AppStateDb, PreferenceWriteOutcome},
    tabs::TabManagerSession,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub const KEY: &str = "instance_sessions";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_INSTANCES: usize = 128;
static LAST_ORDER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ExitOrder {
    when: u64,
    process: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SaveOutcome {
    Saved,
    Superseded,
    Failed,
}

fn wall_time() -> u64 {
    let time =
        unsafe { windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime() };
    ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
}

pub(crate) fn exit_order() -> ExitOrder {
    let now = wall_time();
    let mut previous = LAST_ORDER.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous.saturating_add(1));
        match LAST_ORDER.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => {
                return ExitOrder {
                    when: next,
                    process: std::process::id(),
                }
            }
            Err(actual) => previous = actual,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    version: u32,
    #[serde(default)]
    order: ExitOrder,
    sessions: Vec<TabManagerSession>,
}

pub(crate) fn decode(value: &str) -> Option<Vec<TabManagerSession>> {
    if value.len() > MAX_BYTES {
        return None;
    }
    let record: Record = serde_json::from_str(value).ok()?;
    if record.version != 1 || record.sessions.is_empty() || record.sessions.len() > MAX_INSTANCES {
        return None;
    }
    record
        .sessions
        .into_iter()
        .map(|session| TabManagerSession::from_json(&session.to_json().ok()?))
        .collect()
}

pub(crate) fn save(db: &AppStateDb, sessions: Vec<TabManagerSession>) -> bool {
    save_at(db, sessions, exit_order()) == SaveOutcome::Saved
}

pub(super) fn save_at(
    db: &AppStateDb,
    sessions: Vec<TabManagerSession>,
    order: ExitOrder,
) -> SaveOutcome {
    let Ok(value) = serde_json::to_string(&Record {
        version: 1,
        order,
        sessions,
    }) else {
        return SaveOutcome::Failed;
    };
    if decode(&value).is_none() {
        return SaveOutcome::Failed;
    }
    match db.update_preference_if(KEY, &value, Duration::from_secs(2), |previous| {
        let Some(previous) = previous
            .filter(|value| value.len() <= MAX_BYTES)
            .and_then(|value| serde_json::from_str::<Record>(value).ok())
        else {
            return true;
        };
        if previous.version != 1 {
            return true;
        }
        let now = wall_time();
        // Recover from a clock reset across process launches. Within one
        // process exit_order remains strictly increasing even if UTC moves.
        (previous.order.when > now && order.when <= now) || order >= previous.order
    }) {
        (PreferenceWriteOutcome::Persisted, true) => SaveOutcome::Saved,
        (PreferenceWriteOutcome::Persisted, false) => SaveOutcome::Superseded,
        _ => SaveOutcome::Failed,
    }
}

#[cfg(test)]
mod tests;
