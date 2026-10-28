use super::record;
use crate::{
    app::{tab_transfer, ImageViewerApp},
    tabs::TabManagerSession,
};
use std::{
    io::{self, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
};

struct Startup {
    first: TabManagerSession,
    remaining: Mutex<Option<Vec<TabManagerSession>>>,
    acknowledgement: Mutex<Option<TcpStream>>,
}
static STARTUP: OnceLock<Startup> = OnceLock::new();
static RESTORE_FAILED: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
static RESTORING: AtomicBool = AtomicBool::new(false);

struct Restoring;
impl Drop for Restoring {
    fn drop(&mut self) {
        RESTORING.store(false, Ordering::Release);
    }
}

pub(crate) fn restoring() -> bool {
    RESTORING.load(Ordering::Acquire)
}

pub(super) fn ready() -> bool {
    READY.load(Ordering::Acquire)
}

/// Called under the existing startup mutex. Warm launches restore just the first
/// saved session; only a cold launch may reopen the complete saved group.
pub fn prepare_startup(saved: Option<&str>, location: Option<&str>, multiple: bool) {
    if STARTUP.get().is_some() || tab_transfer::startup_session().is_some() {
        return;
    }
    let Some((first, sessions)) = restoration_plan(saved, location, multiple) else {
        return;
    };
    let _ = STARTUP.set(Startup {
        first,
        remaining: Mutex::new(Some(sessions)),
        acknowledgement: Mutex::new(None),
    });
}

fn restoration_plan(
    saved: Option<&str>,
    location: Option<&str>,
    multiple: bool,
) -> Option<(TabManagerSession, Vec<TabManagerSession>)> {
    if !matches!(location, None | Some("previous_session")) {
        return None;
    }
    let mut sessions = saved.and_then(record::decode)?;
    let first = sessions.remove(0);
    if !multiple {
        sessions.clear();
    }
    Some((first, sessions))
}

pub fn receive_startup(port: u16, token: &str) -> io::Result<()> {
    let (stream, bytes) = tab_transfer::receive_payload(port, token)?;
    let session = std::str::from_utf8(&bytes)
        .ok()
        .and_then(TabManagerSession::from_json)
        .ok_or(io::ErrorKind::InvalidData)?;
    STARTUP
        .set(Startup {
            first: session,
            remaining: Mutex::new(None),
            acknowledgement: Mutex::new(Some(stream)),
        })
        .map_err(|_| io::ErrorKind::AlreadyExists.into())
}

pub(crate) fn startup_session() -> Option<TabManagerSession> {
    STARTUP.get().map(|startup| startup.first.clone())
}

/// Run only after the restored native receiver, tabs and panes are ready. Child
/// processes get exactly one authenticated session, never the whole restart set.
pub(crate) fn confirm_startup(app: &ImageViewerApp) -> bool {
    let Some(startup) = STARTUP.get() else {
        READY.store(true, Ordering::Release);
        crate::infrastructure::windows::system_tray::session_exit::mark_ready();
        return true;
    };
    if app.native_hwnd.is_none() {
        return false;
    }
    if let Some(mut stream) = startup.acknowledgement.lock().unwrap().take() {
        if stream.write_all(b"READY").is_err() {
            return false;
        }
    }
    READY.store(true, Ordering::Release);
    crate::infrastructure::windows::system_tray::session_exit::mark_ready();
    if let Some(remaining) = startup
        .remaining
        .lock()
        .unwrap()
        .take()
        .filter(|sessions| !sessions.is_empty())
    {
        let ctx = app.ui_ctx.clone();
        RESTORING.store(true, Ordering::Release);
        if std::thread::Builder::new()
            .name("restore-instances".into())
            .spawn(move || {
                let _restoring = Restoring;
                for session in remaining {
                    let result = session
                        .to_json()
                        .map_err(io::Error::other)
                        .and_then(|bytes| {
                            tab_transfer::launch_payload("--restore-instance", bytes.as_bytes())
                        });
                    if result.is_err() {
                        // Keep the durable restart set intact for a later cold launch.
                        log::warn!("[SESSION] Could not restore every saved instance");
                        RESTORE_FAILED.store(true, Ordering::Release);
                        ctx.request_repaint();
                        break;
                    }
                }
            })
            .is_err()
        {
            RESTORING.store(false, Ordering::Release);
            RESTORE_FAILED.store(true, Ordering::Release);
        }
    }
    true
}

pub(crate) fn poll_restore_failure() -> bool {
    RESTORE_FAILED.swap(false, Ordering::AcqRel)
}

#[cfg(test)]
mod tests;
