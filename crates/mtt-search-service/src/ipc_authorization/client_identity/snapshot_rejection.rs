//! Negative evidence only. A basename match must NEVER establish MTT identity.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

use super::{ClientIdentityError, EXPECTED_CLIENT_BASENAME};

const MAX_ENTRIES: usize = 65_536;
// Cooperative only: native snapshot/enumeration calls cannot be preempted here.
// The existing connection watchdog still bounds the connection lifecycle.
const SNAPSHOT_BUDGET: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum BasenameEvidence {
    Foreign,
    MatchesExpected,
}

pub(super) fn inspect_pid(pid: u32) -> Result<BasenameEvidence, &'static str> {
    if pid == 0 {
        return Err("snapshot lookup requires a valid client PID");
    }
    let started = Instant::now();
    // One read-only process snapshot, no heaps/modules, privilege changes,
    // retries or PID/name cache. The PID comes from the connected pipe.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|_| "cannot create process snapshot")?;
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.0) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut first = true;
    scan_entries(
        pid,
        MAX_ENTRIES,
        || {
            let result = unsafe {
                if first {
                    first = false;
                    Process32FirstW(HANDLE(handle.as_raw_handle()), &mut entry)
                } else {
                    Process32NextW(HANDLE(handle.as_raw_handle()), &mut entry)
                }
            };
            match result {
                Ok(()) => Ok(entry),
                Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => {
                    Err("client PID not found in process snapshot")
                }
                Err(_) => Err("process snapshot enumeration failed"),
            }
        },
        || started.elapsed() < SNAPSHOT_BUDGET,
    )
}

fn scan_entries(
    pid: u32,
    limit: usize,
    mut next: impl FnMut() -> Result<PROCESSENTRY32W, &'static str>,
    mut within_budget: impl FnMut() -> bool,
) -> Result<BasenameEvidence, &'static str> {
    for _ in 0..limit {
        if !within_budget() {
            return Err("process snapshot budget exceeded");
        }
        let entry = next()?;
        // A slow native call must not become negative evidence after the budget.
        if !within_budget() {
            return Err("process snapshot budget exceeded");
        }
        if entry.th32ProcessID == pid {
            return classify_basename(&entry.szExeFile);
        }
    }
    Err("process snapshot entry limit exceeded")
}

fn classify_basename(encoded: &[u16]) -> Result<BasenameEvidence, &'static str> {
    let length = encoded
        .iter()
        .position(|unit| *unit == 0)
        .ok_or("snapshot basename is unterminated")?;
    if length == 0 || length + 1 == encoded.len() {
        return Err("snapshot basename is missing or possibly truncated");
    }
    let name = String::from_utf16(&encoded[..length])
        .map_err(|_| "snapshot basename is invalid UTF-16")?;
    if name.chars().any(|ch| {
        ch.is_control() || matches!(ch, '\\' | '/' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
    }) || name.ends_with(['.', ' '])
        || name.contains('~')
    {
        // A possible DOS short alias is not proof of a different executable.
        return Err("snapshot basename is not an unambiguous long filename");
    }
    Ok(if name.eq_ignore_ascii_case(EXPECTED_CLIENT_BASENAME) {
        BasenameEvidence::MatchesExpected
    } else {
        BasenameEvidence::Foreign
    })
}

// Returning an error type, not Result<(), _>, makes a trust grant impossible.
pub(super) fn after_image_failure(
    reason: String,
    evidence: Result<BasenameEvidence, &'static str>,
) -> ClientIdentityError {
    match evidence {
        Ok(BasenameEvidence::Foreign) => ClientIdentityError::Rejected(
            "Client process snapshot identifies a different executable basename".into(),
        ),
        Ok(BasenameEvidence::MatchesExpected) => ClientIdentityError::Unavailable(format!(
            "{reason}; process snapshot matched name only; identity remains unavailable"
        )),
        Err(snapshot_error) => ClientIdentityError::Unavailable(format!(
            "{reason}; process snapshot unavailable: {snapshot_error}"
        )),
    }
}

#[cfg(test)]
mod os_tests;
#[cfg(test)]
mod tests;
