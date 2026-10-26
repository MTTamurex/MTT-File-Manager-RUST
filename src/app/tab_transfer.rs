//! Private, authenticated loopback channel for a newly spawned main instance.
//! No paths are placed in command-line arguments or temporary files.
use crate::tabs::{TabManagerSession, TabTransfer};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_BYTES: usize = 1024 * 1024;
static STARTUP: OnceLock<StartupTransfer> = OnceLock::new();

mod scroll_restore;
pub(crate) use scroll_restore::restore_ready_scroll;

struct StartupTransfer {
    transfer: TabTransfer,
    stream: Mutex<Option<TcpStream>>,
}

pub(crate) fn startup_session() -> Option<TabManagerSession> {
    STARTUP
        .get()
        .map(|startup| startup.transfer.session.clone())
}

pub(crate) fn restore_browser_state(app: &mut crate::app::ImageViewerApp) {
    if let Some(startup) = STARTUP.get() {
        startup.transfer.restore(app);
        scroll_restore::remember(app);
    }
}

/// Called by the CLI before starting the UI. Invalid transfers must not fall
/// through to ordinary startup or restore an unrelated saved session.
pub fn receive_startup(port: u16, token: &str) -> io::Result<()> {
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut stream = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let transfer = read_transfer(&mut stream, token)?;
    STARTUP
        .set(StartupTransfer {
            transfer,
            stream: Mutex::new(Some(stream)),
        })
        .map_err(|_| io::ErrorKind::AlreadyExists)?;
    Ok(())
}

fn read_transfer(stream: &mut TcpStream, token: &str) -> io::Result<TabTransfer> {
    stream.write_all(token.as_bytes())?;
    let mut size = [0; 4];
    stream.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size == 0 || size > MAX_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    std::str::from_utf8(&bytes)
        .ok()
        .and_then(TabTransfer::from_json)
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
}

/// Acknowledge only after the native window and its requested session exist.
pub(crate) fn confirm_startup(app: &crate::app::ImageViewerApp) -> bool {
    let Some(startup) = STARTUP.get() else {
        return true;
    };
    let Some(mut stream) = startup.stream.lock().unwrap().take() else {
        return true;
    };
    let ready = app.native_hwnd.is_some() && startup.transfer.locations_match(&app.tab_manager);
    let sent = stream
        .write_all(if ready { b"READY" } else { b"ERROR" })
        .is_ok();
    ready && sent
}

pub(crate) fn launch(transfer: &TabTransfer) -> io::Result<()> {
    let bytes = serde_json::to_vec(transfer).map_err(io::Error::other)?;
    if bytes.len() > MAX_BYTES {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let token = format!(
        "{:032x}",
        unsafe { windows::Win32::System::Com::CoCreateGuid() }
            .map_err(io::Error::other)?
            .to_u128()
    );
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--detach-tab")
        .arg(listener.local_addr()?.port().to_string())
        .arg(&token)
        .spawn()?;
    let result = send_session(&listener, &mut child, token.as_bytes(), &bytes);
    if result.is_err() {
        // This process was created exclusively for this transfer. Never leave
        // an unacknowledged duplicate alive after retaining the original tab.
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn send_session(
    listener: &TcpListener,
    child: &mut Child,
    token: &[u8],
    bytes: &[u8],
) -> io::Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if Instant::now() >= deadline || child.try_wait()?.is_some() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                if !authenticate_stream(&mut stream, token, deadline).unwrap_or(false) {
                    continue;
                }
                return send_payload(&mut stream, bytes, remaining(deadline)?);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error),
        }
    }
}

fn authenticate_stream(
    stream: &mut TcpStream,
    token: &[u8],
    deadline: Instant,
) -> io::Result<bool> {
    // Winsock inherits the listener's nonblocking mode on accepted sockets.
    // The listener is polled, but the authenticated stream uses blocking I/O
    // with deadlines; otherwise a normal startup delay becomes WSAEWOULDBLOCK.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(remaining(deadline)?.min(Duration::from_secs(2))))?;
    let mut supplied = [0; 32];
    stream.read_exact(&mut supplied)?;
    Ok(supplied == token)
}

fn send_payload(stream: &mut TcpStream, bytes: &[u8], timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    stream.set_write_timeout(Some(remaining(deadline)?))?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.set_write_timeout(Some(remaining(deadline)?))?;
    stream.write_all(bytes)?;
    stream.set_read_timeout(Some(remaining(deadline)?))?;
    let mut acknowledgement = [0; 5];
    stream.read_exact(&mut acknowledgement)?;
    if acknowledgement == *b"READY" {
        Ok(())
    } else {
        Err(io::ErrorKind::InvalidData.into())
    }
}

fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::ErrorKind::TimedOut.into())
}

#[cfg(test)]
mod tests;
