//! Centralized FFmpeg Process Controller
//!
//! This module provides a thread-safe controller for FFmpeg processes.
//! It ensures that at most ONE FFmpeg process exists per session,
//! with explicit lifecycle management (spawn/kill/wait).
//!
//! # Safety
//! - All spawns and kills are serialized via Mutex
//! - PIDs are registered globally for fail-safe cleanup
//! - Drop implementation ensures cleanup even on panic

use std::collections::HashSet;
use std::io::{self, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

// ============================================================================
// GLOBAL PID REGISTRY - Fail-safe cleanup
// ============================================================================

static GLOBAL_PID_REGISTRY: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();

fn get_registry() -> &'static Mutex<HashSet<u32>> {
    GLOBAL_PID_REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Register a PID in the global registry
fn register_pid(pid: u32) {
    if let Ok(mut registry) = get_registry().lock() {
        registry.insert(pid);
        println!(
            "[FFmpegSession] Registered PID {} (active: {})",
            pid,
            registry.len()
        );
    }
}

/// Unregister a PID from the global registry
fn unregister_pid(pid: u32) {
    if let Ok(mut registry) = get_registry().lock() {
        registry.remove(&pid);
        println!(
            "[FFmpegSession] Unregistered PID {} (active: {})",
            pid,
            registry.len()
        );
    }
}

/// Kill ALL registered FFmpeg processes - call on app shutdown
pub fn kill_all_ffmpeg_processes() {
    let pids: Vec<u32> = {
        match get_registry().lock() {
            Ok(registry) => registry.iter().copied().collect(),
            Err(_) => return,
        }
    };

    if pids.is_empty() {
        println!("[FFmpegSession] No active FFmpeg processes to kill");
        return;
    }

    println!(
        "[FFmpegSession] Killing {} orphan FFmpeg process(es)...",
        pids.len()
    );

    for pid in pids {
        kill_process_by_pid(pid);
        unregister_pid(pid);
    }
}

/// Force kill a process by PID (Windows)
#[cfg(target_os = "windows")]
fn kill_process_by_pid(pid: u32) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new("taskkill");
    cmd.args(&["/F", "/PID", &pid.to_string()]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                println!("[FFmpegSession] Force-killed PID {}", pid);
            }
        }
        Err(e) => {
            eprintln!("[FFmpegSession] Failed to kill PID {}: {}", pid, e);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn kill_process_by_pid(pid: u32) {
    use std::os::unix::process::CommandExt;
    let _ = Command::new("kill")
        .args(&["-9", &pid.to_string()])
        .status();
}

// ============================================================================
// FFMPEG SESSION - Central Process Controller
// ============================================================================

/// Internal state for FFmpeg session
struct FfmpegSessionInner {
    child: Option<Child>,
    pid: Option<u32>,
    stderr_reader: Option<std::process::ChildStderr>,
}

impl Default for FfmpegSessionInner {
    fn default() -> Self {
        Self {
            child: None,
            pid: None,
            stderr_reader: None,
        }
    }
}

/// Thread-safe FFmpeg process controller.
///
/// Ensures at most ONE FFmpeg process per session.
/// Seek operations must call `kill()` before `spawn()`.
#[derive(Clone)]
pub struct FfmpegSession {
    inner: Arc<Mutex<FfmpegSessionInner>>,
}

impl FfmpegSession {
    /// Create a new FFmpeg session controller
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FfmpegSessionInner::default())),
        }
    }

    /// Check if a process is currently running
    pub fn is_running(&self) -> bool {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(ref mut child) = inner.child {
                match child.try_wait() {
                    Ok(None) => return true, // Still running
                    Ok(Some(_)) => {
                        // Process exited, clean up
                        if let Some(pid) = inner.pid.take() {
                            unregister_pid(pid);
                        }
                        inner.child = None;
                        return false;
                    }
                    Err(_) => return false,
                }
            }
        }
        false
    }

    /// Kill the current FFmpeg process with explicit wait.
    ///
    /// This method:
    /// 1. Sends kill signal
    /// 2. Waits up to 5 seconds for graceful exit
    /// 3. Force-kills via taskkill if needed
    /// 4. Unregisters PID from global registry
    pub fn kill(&self, reason: &str) {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(), // Recover from panic
        };

        let pid = match inner.pid {
            Some(p) => p,
            None => return, // No process to kill
        };

        println!("[FFmpegSession] Killing PID {} (reason: {})", pid, reason);

        if let Some(ref mut child) = inner.child {
            // Step 1: Send kill signal
            let _ = child.kill();

            // Step 2: Wait with timeout (5 seconds)
            let start = Instant::now();
            let timeout = Duration::from_secs(5);

            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        println!("[FFmpegSession] PID {} exited gracefully: {}", pid, status);
                        break;
                    }
                    Ok(None) => {
                        if start.elapsed() > timeout {
                            // Step 3: Force kill
                            println!("[FFmpegSession] PID {} timeout, force killing...", pid);
                            kill_process_by_pid(pid);
                            let _ = child.wait(); // Reap zombie
                            break;
                        }
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        eprintln!("[FFmpegSession] Wait error for PID {}: {}", pid, e);
                        kill_process_by_pid(pid);
                        break;
                    }
                }
            }
        }

        // Step 4: Cleanup
        unregister_pid(pid);
        inner.child = None;
        inner.pid = None;
        inner.stderr_reader = None;
    }

    /// Spawn a new FFmpeg process.
    ///
    /// **IMPORTANT**: Call `kill()` first if a process might be running!
    ///
    /// Returns the stdout pipe for reading transcoded data.
    pub fn spawn(&self, args: Vec<String>) -> io::Result<ChildStdout> {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        // Safety check: kill existing process first
        if inner.child.is_some() {
            drop(inner);
            self.kill("spawn called with existing process");
            inner = self.inner.lock().unwrap();
        }

        // Build command
        let mut cmd = Command::new("ffmpeg");
        cmd.args(&args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        // Spawn
        let mut child = cmd.spawn()?;
        let pid = child.id();

        // Extract stdout
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Failed to capture stdout"))?;

        // Store stderr for debugging
        inner.stderr_reader = child.stderr.take();

        // Register and store
        register_pid(pid);
        inner.pid = Some(pid);
        inner.child = Some(child);

        println!("[FFmpegSession] Spawned new FFmpeg PID {}", pid);

        Ok(stdout)
    }

    /// Get stderr output (for debugging failed processes)
    pub fn read_stderr(&self) -> Option<String> {
        let mut inner = match self.inner.lock() {
            Ok(guard) => guard,
            Err(_) => return None,
        };

        if let Some(ref mut stderr) = inner.stderr_reader {
            let mut buf = String::new();
            if stderr.read_to_string(&mut buf).is_ok() {
                return Some(buf);
            }
        }
        None
    }

    /// Get current PID if running
    pub fn pid(&self) -> Option<u32> {
        self.inner.lock().ok().and_then(|inner| inner.pid)
    }
}

impl Drop for FfmpegSession {
    fn drop(&mut self) {
        // Only kill if we're the last owner of this Arc
        if Arc::strong_count(&self.inner) == 1 {
            self.kill("session dropped");
        }
    }
}

impl Default for FfmpegSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_lifecycle() {
        let session = FfmpegSession::new();
        assert!(!session.is_running());
        assert!(session.pid().is_none());
    }
}
