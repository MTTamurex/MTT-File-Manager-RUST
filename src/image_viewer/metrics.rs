// ── Resource leak diagnostics ───────────────────────────────────────────

const IDLE_WORKING_SET_TRIM_AFTER: std::time::Duration = std::time::Duration::from_secs(4);
const WORKING_SET_TRIM_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
const WORKING_SET_TRIM_MIN_BYTES: u64 = 24 * 1024 * 1024;
const WORKING_SET_TRIM_DELAYS: [std::time::Duration; 3] = [
    std::time::Duration::from_millis(750),
    std::time::Duration::from_millis(2500),
    std::time::Duration::from_millis(6000),
];

pub fn working_set_trim_min_bytes() -> u64 {
    WORKING_SET_TRIM_MIN_BYTES
}

#[derive(Clone, Copy)]
struct ProcessMemoryUsage {
    working_set_bytes: u64,
    private_usage_bytes: u64,
}

#[cfg(target_os = "windows")]
fn capture_process_memory_usage() -> ProcessMemoryUsage {
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let has_memory_info = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        )
        .as_bool()
    };
    ProcessMemoryUsage {
        working_set_bytes: if has_memory_info {
            counters.WorkingSetSize as u64
        } else {
            0
        },
        private_usage_bytes: if has_memory_info {
            counters.PrivateUsage as u64
        } else {
            0
        },
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_process_memory_usage() -> ProcessMemoryUsage {
    ProcessMemoryUsage {
        working_set_bytes: 0,
        private_usage_bytes: 0,
    }
}

pub fn current_working_set_bytes() -> u64 {
    capture_process_memory_usage().working_set_bytes
}

pub fn idle_trim_is_due(
    now: std::time::Instant,
    last_activity: std::time::Instant,
    last_trim_request: std::time::Instant,
    working_set_bytes: u64,
    has_pending_work: bool,
) -> bool {
    !has_pending_work
        && working_set_bytes >= WORKING_SET_TRIM_MIN_BYTES
        && now.duration_since(last_activity) >= IDLE_WORKING_SET_TRIM_AFTER
        && now.duration_since(last_trim_request) >= WORKING_SET_TRIM_MIN_INTERVAL
}

pub fn working_set_trim_cancelled(current_generation: u64, scheduled_generation: u64) -> bool {
    current_generation != scheduled_generation
}

pub fn request_idle_working_set_trim(
    activity_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    scheduled_generation: u64,
) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::thread::Builder::new()
            .name("image-viewer-ws-trim".to_string())
            .stack_size(128 * 1024)
            .spawn(move || {
                trim_working_set_series(activity_generation, scheduled_generation);
            })
            .is_ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (activity_generation, scheduled_generation);
        false
    }
}

#[cfg(target_os = "windows")]
fn trim_working_set_series(
    activity_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    scheduled_generation: u64,
) {
    use std::sync::atomic::Ordering;
    use windows::Win32::System::Memory::{
        SetProcessWorkingSetSizeEx, SETPROCESSWORKINGSETSIZEEX_FLAGS,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut elapsed = std::time::Duration::ZERO;
    for delay in WORKING_SET_TRIM_DELAYS {
        if delay > elapsed {
            std::thread::sleep(delay - elapsed);
            elapsed = delay;
        }
        if working_set_trim_cancelled(
            activity_generation.load(Ordering::Acquire),
            scheduled_generation,
        ) {
            return;
        }

        let before = capture_resource_snapshot();
        let result = unsafe {
            SetProcessWorkingSetSizeEx(
                GetCurrentProcess(),
                usize::MAX,
                usize::MAX,
                SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
            )
        };
        let after = capture_resource_snapshot();
        if result.is_ok() {
            crate::infrastructure::diagnostic_logger::diag_info(
                "image_viewer_memory",
                "working_set_trim",
                &[
                    crate::infrastructure::diagnostic_logger::field_u64(
                        "pid",
                        std::process::id() as u64,
                    ),
                    crate::infrastructure::diagnostic_logger::field_u64(
                        "working_set_before_bytes",
                        before.working_set_bytes,
                    ),
                    crate::infrastructure::diagnostic_logger::field_u64(
                        "working_set_after_bytes",
                        after.working_set_bytes,
                    ),
                    crate::infrastructure::diagnostic_logger::field_u64(
                        "private_usage_bytes",
                        after.private_usage_bytes,
                    ),
                ],
            );
        }
    }
}

/// Snapshot of process-level resource counters.
/// Used to detect silent handle/thread/GDI leaks that cause system-wide
/// slowdown without obvious CPU/memory spikes in Task Manager.
#[cfg(test)]
mod tests {
    use super::{idle_trim_is_due, working_set_trim_cancelled};
    use std::time::{Duration, Instant};

    #[test]
    fn idle_trim_waits_for_idle_and_no_pending_work() {
        let now = Instant::now();
        let old = now - Duration::from_secs(20);

        assert!(idle_trim_is_due(now, old, old, 32 * 1024 * 1024, false));
        assert!(!idle_trim_is_due(
            now,
            now - Duration::from_secs(3),
            old,
            32 * 1024 * 1024,
            false
        ));
        assert!(!idle_trim_is_due(now, old, old, 32 * 1024 * 1024, true));
        assert!(!idle_trim_is_due(now, old, old, 23 * 1024 * 1024, false));
        assert!(!idle_trim_is_due(
            now,
            old,
            now - Duration::from_secs(9),
            32 * 1024 * 1024,
            false
        ));
        assert!(!working_set_trim_cancelled(4, 4));
        assert!(working_set_trim_cancelled(5, 4));
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResourceSnapshot {
    pub handle_count: u32,
    pub gdi_objects: u32,
    pub user_objects: u32,
    pub thread_count: u32,
    pub working_set_bytes: u64,
    pub private_usage_bytes: u64,
}

impl std::fmt::Display for ResourceSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "handles={} gdi={} user={} threads={} working_set_mb={:.1} private_mb={:.1}",
            self.handle_count,
            self.gdi_objects,
            self.user_objects,
            self.thread_count,
            self.working_set_bytes as f64 / 1_048_576.0,
            self.private_usage_bytes as f64 / 1_048_576.0
        )
    }
}

#[cfg(target_os = "windows")]
pub fn capture_resource_snapshot() -> ResourceSnapshot {
    let resources = crate::infrastructure::windows::get_current_process_kernel_resources();
    let memory = capture_process_memory_usage();
    ResourceSnapshot {
        handle_count: resources.handle_count,
        gdi_objects: resources.gdi_objects,
        user_objects: resources.user_objects,
        thread_count: resources.thread_count,
        working_set_bytes: memory.working_set_bytes,
        private_usage_bytes: memory.private_usage_bytes,
    }
}

#[cfg(not(target_os = "windows"))]
pub fn capture_resource_snapshot() -> ResourceSnapshot {
    ResourceSnapshot {
        handle_count: 0,
        gdi_objects: 0,
        user_objects: 0,
        thread_count: 0,
        working_set_bytes: 0,
        private_usage_bytes: 0,
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
        crate::infrastructure::diagnostic_logger::diag_info(
            "image_viewer_memory",
            "snapshot",
            &[
                crate::infrastructure::diagnostic_logger::field_u64(
                    "pid",
                    std::process::id() as u64,
                ),
                crate::infrastructure::diagnostic_logger::field_u64(
                    "working_set_bytes",
                    snap.working_set_bytes,
                ),
                crate::infrastructure::diagnostic_logger::field_u64(
                    "private_usage_bytes",
                    snap.private_usage_bytes,
                ),
                crate::infrastructure::diagnostic_logger::field_u64(
                    "handle_count",
                    snap.handle_count as u64,
                ),
                crate::infrastructure::diagnostic_logger::field_u64(
                    "gdi_objects",
                    snap.gdi_objects as u64,
                ),
                crate::infrastructure::diagnostic_logger::field_u64(
                    "user_objects",
                    snap.user_objects as u64,
                ),
                crate::infrastructure::diagnostic_logger::field_u64(
                    "thread_count",
                    snap.thread_count as u64,
                ),
            ],
        );

        if self.baseline.is_none() {
            self.baseline = Some(snap);
            log::info!(
                "[IMAGE-VIEWER][RESOURCE-MONITOR] pid={} baseline: {}",
                std::process::id(),
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
                "[IMAGE-VIEWER][RESOURCE-MONITOR] pid={} GROWTH DETECTED: {} | \
                 delta_from_baseline: handles={:+} gdi={:+} threads={:+} | \
                 delta_from_prev: handles={:+}",
                std::process::id(),
                snap,
                delta_handles,
                delta_gdi,
                delta_threads,
                delta_handles_prev,
            );
        } else {
            log::info!(
                "[IMAGE-VIEWER][RESOURCE-MONITOR] pid={} {}  | delta_baseline: h={:+} g={:+} t={:+}",
                std::process::id(),
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
