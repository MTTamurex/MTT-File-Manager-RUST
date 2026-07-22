/// Trim the process working set after large one-shot indexing operations.
///
/// The in-memory index remains intact. This only asks Windows to remove cold
/// resident pages from the process working set, which is the number shown in
/// Task Manager's "Memory" column. Set `MTT_SEARCH_DISABLE_WS_TRIM=1` to disable.
fn operation_lock() -> &'static parking_lot::RwLock<()> {
    static LOCK: std::sync::OnceLock<parking_lot::RwLock<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| parking_lot::RwLock::new(()))
}

pub(crate) struct ActiveOperationGuard {
    _guard: parking_lot::RwLockReadGuard<'static, ()>,
}

/// Prevent working-set trims while an IPC request is actively using the index.
pub(crate) fn begin_active_operation() -> ActiveOperationGuard {
    ActiveOperationGuard {
        _guard: operation_lock().read(),
    }
}

pub(crate) fn trim_working_set(reason: &str) {
    if trim_disabled() {
        return;
    }

    let Some(_guard) = operation_lock().try_write() else {
        return;
    };
    trim_working_set_uncoordinated(reason);
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

/// Process-wide throttle for idle-triggered trims. Stores the elapsed
/// milliseconds (since the process-local base instant) of the last idle trim.
/// `u64::MAX` means "never trimmed yet".
static LAST_IDLE_TRIM_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(u64::MAX);

fn idle_trim_base() -> std::time::Instant {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *BASE.get_or_init(std::time::Instant::now)
}

/// Trim the working set during idle periods, throttled process-wide so that
/// concurrent volume indexer threads do not repeatedly trim within the same
/// window. Returns `true` when a trim was actually performed.
///
/// This reclaims working-set growth caused by on-demand work (searches and
/// folder-size traversals page in the name arena and record data) that would
/// otherwise stay resident because the periodic persist trim only runs when the
/// index is dirty.
pub(crate) fn trim_working_set_idle(reason: &str, min_interval: std::time::Duration) -> bool {
    use std::sync::atomic::Ordering;

    if trim_disabled() {
        return false;
    }

    let Some(_guard) = operation_lock().try_write() else {
        return false;
    };

    let now_ms = idle_trim_base().elapsed().as_millis().min(u64::MAX as u128) as u64;
    let interval_ms = min_interval.as_millis().min(u64::MAX as u128) as u64;

    let last = LAST_IDLE_TRIM_MS.load(Ordering::Relaxed);
    if last != u64::MAX && now_ms.saturating_sub(last) < interval_ms {
        return false;
    }

    // Claim the window atomically so only one thread trims per interval.
    if LAST_IDLE_TRIM_MS
        .compare_exchange(last, now_ms, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return false;
    }

    trim_working_set_uncoordinated(reason);
    true
}

pub(crate) fn trim_working_set_delayed(reason: String, delay: std::time::Duration) {
    if trim_disabled() {
        return;
    }

    let spawn_result = std::thread::Builder::new()
        .name("working-set-trim".to_string())
        .spawn(move || {
            std::thread::sleep(delay);
            trim_working_set(&reason);
        });

    if let Err(error) = spawn_result {
        eprintln!("[MEM] Failed to spawn delayed working set trim: {}", error);
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
