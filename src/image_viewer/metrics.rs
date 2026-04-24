// ── Resource leak diagnostics ───────────────────────────────────────────

/// Snapshot of process-level resource counters.
/// Used to detect silent handle/thread/GDI leaks that cause system-wide
/// slowdown without obvious CPU/memory spikes in Task Manager.
#[derive(Debug, Clone, Copy)]
pub struct ResourceSnapshot {
    pub handle_count: u32,
    pub gdi_objects: u32,
    pub user_objects: u32,
    pub thread_count: u32,
}

impl std::fmt::Display for ResourceSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "handles={} gdi={} user={} threads={}",
            self.handle_count, self.gdi_objects, self.user_objects, self.thread_count
        )
    }
}

#[cfg(target_os = "windows")]
pub fn capture_resource_snapshot() -> ResourceSnapshot {
    use windows::Win32::System::Threading::GetCurrentProcess;

    // These APIs may not be in the windows crate feature set, so use raw FFI.
    extern "system" {
        fn GetProcessHandleCount(
            hProcess: *mut std::ffi::c_void,
            pdwHandleCount: *mut u32,
        ) -> i32;
        fn GetGuiResources(hProcess: *mut std::ffi::c_void, uiFlags: u32) -> u32;
    }

    const GR_GDIOBJECTS: u32 = 0;
    const GR_USEROBJECTS: u32 = 1;

    unsafe {
        let process = GetCurrentProcess();

        let mut handle_count: u32 = 0;
        GetProcessHandleCount(process.0, &mut handle_count);

        let gdi_objects = GetGuiResources(process.0, GR_GDIOBJECTS);
        let user_objects = GetGuiResources(process.0, GR_USEROBJECTS);

        let thread_count = count_process_threads();

        ResourceSnapshot {
            handle_count,
            gdi_objects,
            user_objects,
            thread_count,
        }
    }
}

#[cfg(target_os = "windows")]
fn count_process_threads() -> u32 {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };

    let pid = std::process::id();
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    let Ok(snapshot) = snapshot else {
        return 0;
    };

    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };

    let mut count = 0u32;
    unsafe {
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == pid {
                    count += 1;
                }
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    count
}

#[cfg(not(target_os = "windows"))]
pub fn capture_resource_snapshot() -> ResourceSnapshot {
    ResourceSnapshot {
        handle_count: 0,
        gdi_objects: 0,
        user_objects: 0,
        thread_count: 0,
    }
}

/// Periodic resource leak monitor. Call from the UI update loop.
/// Logs a warning every `interval` when resource counts are growing.
pub struct ResourceLeakMonitor {
    last_log: std::time::Instant,
    interval: std::time::Duration,
    baseline: Option<ResourceSnapshot>,
    prev: Option<ResourceSnapshot>,
}

impl ResourceLeakMonitor {
    pub fn new(interval: std::time::Duration) -> Self {
        Self {
            last_log: std::time::Instant::now() - interval, // trigger on first call
            interval,
            baseline: None,
            prev: None,
        }
    }

    /// Call each frame. Returns `Some(snapshot)` when a log was emitted.
    pub fn tick(&mut self) -> Option<ResourceSnapshot> {
        if self.last_log.elapsed() < self.interval {
            return None;
        }
        self.last_log = std::time::Instant::now();

        let snap = capture_resource_snapshot();

        if self.baseline.is_none() {
            self.baseline = Some(snap);
            log::info!(
                "[IMAGE-VIEWER][RESOURCE-MONITOR] baseline: {}",
                snap
            );
            self.prev = Some(snap);
            return Some(snap);
        }

        let baseline = self.baseline.unwrap();
        let prev = self.prev.unwrap_or(baseline);

        // Log delta from baseline and from previous snapshot
        let delta_handles = snap.handle_count as i64 - baseline.handle_count as i64;
        let delta_gdi = snap.gdi_objects as i64 - baseline.gdi_objects as i64;
        let delta_threads = snap.thread_count as i64 - baseline.thread_count as i64;
        let delta_handles_prev = snap.handle_count as i64 - prev.handle_count as i64;

        // Warn if handles grew significantly since baseline
        if delta_handles > 50 || delta_gdi > 20 || delta_threads > 8 {
            log::warn!(
                "[IMAGE-VIEWER][RESOURCE-MONITOR] GROWTH DETECTED: {} | \
                 delta_from_baseline: handles={:+} gdi={:+} threads={:+} | \
                 delta_from_prev: handles={:+}",
                snap,
                delta_handles,
                delta_gdi,
                delta_threads,
                delta_handles_prev,
            );
        } else {
            log::info!(
                "[IMAGE-VIEWER][RESOURCE-MONITOR] {}  | delta_baseline: h={:+} g={:+} t={:+}",
                snap,
                delta_handles,
                delta_gdi,
                delta_threads,
            );
        }

        self.prev = Some(snap);
        Some(snap)
    }
}

