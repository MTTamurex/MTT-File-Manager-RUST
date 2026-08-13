use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const INACTIVITY_DELAY: Duration = Duration::from_secs(10);
const PERIODIC_TRIM_INTERVAL: Duration = Duration::from_secs(120);
const CONTENTION_RETRY_DELAY: Duration = Duration::from_millis(500);
const SHUTDOWN_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Trimming only evicts cold resident pages from the process working set. The
/// in-memory index remains intact and is paged back in by Windows when needed.
fn operation_lock() -> &'static parking_lot::RwLock<()> {
    static LOCK: OnceLock<parking_lot::RwLock<()>> = OnceLock::new();
    LOCK.get_or_init(|| parking_lot::RwLock::new(()))
}

struct TrimSchedule {
    requested_deadline: Option<Instant>,
    periodic_deadline: Instant,
    retry_not_before: Option<Instant>,
    reason: String,
}

impl TrimSchedule {
    fn new(now: Instant) -> Self {
        Self {
            requested_deadline: None,
            periodic_deadline: now + PERIODIC_TRIM_INTERVAL,
            retry_not_before: None,
            reason: "periodic idle maintenance".to_string(),
        }
    }

    fn record_activity(&mut self, now: Instant, reason: String) {
        let deadline = now + INACTIVITY_DELAY;
        self.requested_deadline = Some(deadline);
        self.retry_not_before = None;
        self.reason = reason;
    }

    fn base_deadline(&self) -> Instant {
        self.requested_deadline
            .map_or(self.periodic_deadline, |deadline| {
                deadline.min(self.periodic_deadline)
            })
    }

    fn next_deadline(&self) -> Instant {
        self.retry_not_before.map_or_else(
            || self.base_deadline(),
            |retry| self.base_deadline().max(retry),
        )
    }

    fn is_due(&self, now: Instant) -> bool {
        now >= self.next_deadline()
    }

    fn defer_for_contention(&mut self, now: Instant) {
        self.retry_not_before = Some(now + CONTENTION_RETRY_DELAY);
    }

    fn complete(&mut self, now: Instant) {
        self.requested_deadline = None;
        self.periodic_deadline = now + PERIODIC_TRIM_INTERVAL;
        self.retry_not_before = None;
        self.reason = "periodic idle maintenance".to_string();
    }
}

struct TrimCoordinator {
    schedule: parking_lot::Mutex<TrimSchedule>,
    wake: parking_lot::Condvar,
}

fn coordinator() -> &'static Arc<TrimCoordinator> {
    static COORDINATOR: OnceLock<Arc<TrimCoordinator>> = OnceLock::new();
    COORDINATOR.get_or_init(|| {
        Arc::new(TrimCoordinator {
            schedule: parking_lot::Mutex::new(TrimSchedule::new(Instant::now())),
            wake: parking_lot::Condvar::new(),
        })
    })
}

/// Start the single process-wide worker that coalesces and retries trims.
pub(crate) fn start_coordinator(shutdown: Arc<AtomicBool>) {
    static STARTED: AtomicBool = AtomicBool::new(false);

    if trim_disabled() || STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    let coordinator = coordinator().clone();
    let spawn_result = std::thread::Builder::new()
        .name("working-set-trim".to_string())
        .spawn(move || run_coordinator(coordinator, shutdown));

    match spawn_result {
        Ok(_) => eprintln!(
            "[MEM] Working set trim coordinator started (idle={}s, periodic={}s)",
            INACTIVITY_DELAY.as_secs(),
            PERIODIC_TRIM_INTERVAL.as_secs()
        ),
        Err(error) => {
            STARTED.store(false, Ordering::Release);
            eprintln!(
                "[MEM] Failed to spawn working set trim coordinator: {}",
                error
            );
        }
    }
}

fn run_coordinator(coordinator: Arc<TrimCoordinator>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Relaxed) {
        let mut schedule = coordinator.schedule.lock();
        let now = Instant::now();
        let wait = schedule
            .next_deadline()
            .saturating_duration_since(now)
            .min(SHUTDOWN_CHECK_INTERVAL);

        if !wait.is_zero() {
            coordinator.wake.wait_for(&mut schedule, wait);
            continue;
        }
        drop(schedule);

        let Some(_operation_guard) = operation_lock().try_write() else {
            let mut schedule = coordinator.schedule.lock();
            if schedule.is_due(Instant::now()) {
                schedule.defer_for_contention(Instant::now());
            }
            continue;
        };

        // Activity may have moved the deadline while this worker was waiting
        // for the operation lock, so validate it again before trimming.
        let reason = {
            let mut schedule = coordinator.schedule.lock();
            let now = Instant::now();
            if !schedule.is_due(now) {
                continue;
            }

            let reason = schedule.reason.clone();
            schedule.complete(now);
            reason
        };
        trim_working_set_uncoordinated(&reason);
    }
}

pub(crate) struct ActiveOperationGuard {
    trim_reason: Option<&'static str>,
    guard: Option<parking_lot::RwLockReadGuard<'static, ()>>,
}

/// Prevent working-set trims while an IPC request is actively using the index.
pub(crate) fn begin_active_operation(reason: &'static str) -> ActiveOperationGuard {
    ActiveOperationGuard {
        trim_reason: Some(reason),
        guard: Some(operation_lock().read()),
    }
}

/// Block a concurrent trim during recurring internal maintenance without
/// extending the user-facing inactivity deadline.
pub(crate) fn begin_trim_exclusion() -> ActiveOperationGuard {
    ActiveOperationGuard {
        trim_reason: None,
        guard: Some(operation_lock().read()),
    }
}

impl Drop for ActiveOperationGuard {
    fn drop(&mut self) {
        self.guard.take();
        if let Some(reason) = self.trim_reason {
            request_trim(reason);
        }
    }
}

/// Request one coalesced trim after the process has remained idle for 10 seconds.
pub(crate) fn request_trim(reason: impl Into<String>) {
    if trim_disabled() {
        return;
    }

    let coordinator = coordinator();
    coordinator
        .schedule
        .lock()
        .record_activity(Instant::now(), reason.into());
    coordinator.wake.notify_one();
}

fn trim_working_set_uncoordinated(reason: &str) {
    unsafe {
        libmimalloc_sys::mi_collect(true);
    }

    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Memory::{
            SetProcessWorkingSetSizeEx, SETPROCESSWORKINGSETSIZEEX_FLAGS,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;

        let process = GetCurrentProcess();
        match SetProcessWorkingSetSizeEx(
            process,
            usize::MAX,
            usize::MAX,
            SETPROCESSWORKINGSETSIZEEX_FLAGS(0),
        ) {
            Ok(()) => eprintln!("[MEM] Trimmed working set after {}", reason),
            Err(error) => eprintln!("[MEM] Working set trim failed after {}: {}", reason, error),
        }
    }
}

fn trim_disabled() -> bool {
    match std::env::var("MTT_SEARCH_DISABLE_WS_TRIM") {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_schedules_trim_after_ten_seconds() {
        let now = Instant::now();
        let mut schedule = TrimSchedule::new(now);

        schedule.record_activity(now, "search".to_string());

        assert_eq!(schedule.next_deadline(), now + INACTIVITY_DELAY);
        assert!(!schedule.is_due(now + INACTIVITY_DELAY - Duration::from_millis(1)));
        assert!(schedule.is_due(now + INACTIVITY_DELAY));
    }

    #[test]
    fn newer_activity_extends_the_idle_deadline() {
        let now = Instant::now();
        let mut schedule = TrimSchedule::new(now);
        schedule.record_activity(now, "search".to_string());

        schedule.record_activity(now + Duration::from_secs(8), "folder size".to_string());

        assert_eq!(
            schedule.next_deadline(),
            now + Duration::from_secs(8) + INACTIVITY_DELAY
        );
        assert_eq!(schedule.reason, "folder size");
    }

    #[test]
    fn periodic_deadline_caps_continuous_activity() {
        let now = Instant::now();
        let mut schedule = TrimSchedule::new(now);

        for seconds in (0..=180).step_by(5) {
            schedule.record_activity(
                now + Duration::from_secs(seconds),
                "continuous activity".to_string(),
            );
        }

        assert_eq!(schedule.next_deadline(), now + PERIODIC_TRIM_INTERVAL);
        assert!(schedule.is_due(now + PERIODIC_TRIM_INTERVAL));
    }

    #[test]
    fn contention_keeps_the_trim_pending() {
        let now = Instant::now();
        let mut schedule = TrimSchedule::new(now);
        schedule.record_activity(now, "search".to_string());
        let due = now + INACTIVITY_DELAY;

        schedule.defer_for_contention(due);

        assert!(!schedule.is_due(due));
        assert_eq!(schedule.next_deadline(), due + CONTENTION_RETRY_DELAY);
        assert!(schedule.is_due(due + CONTENTION_RETRY_DELAY));
        assert_eq!(schedule.requested_deadline, Some(due));
    }

    #[test]
    fn completion_restores_periodic_fallback() {
        let now = Instant::now();
        let mut schedule = TrimSchedule::new(now);
        schedule.record_activity(now, "search".to_string());
        let completed_at = now + INACTIVITY_DELAY;

        schedule.complete(completed_at);

        assert_eq!(schedule.requested_deadline, None);
        assert_eq!(
            schedule.next_deadline(),
            completed_at + PERIODIC_TRIM_INTERVAL
        );
    }
}
