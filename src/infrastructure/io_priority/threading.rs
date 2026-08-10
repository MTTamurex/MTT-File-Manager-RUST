use std::cell::Cell;

use super::IOPriority;

thread_local! {
    static THREAD_BG_MODE_ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// PERF-03: memo of the priority class last applied to this thread.
    /// Thumbnail workers pop thousands of requests with identical priorities
    /// back-to-back; skipping redundant SetThreadPriority syscalls removes
    /// that per-request kernel churn.
    static LAST_PRIORITY_CLASS: Cell<Option<IOPriority>> = const { Cell::new(None) };
}

/// Set the current thread's priority based on I/O priority level.
pub(super) fn set_thread_priority(priority: IOPriority) {
    use windows::Win32::System::Threading::*;

    // PERF-03: early-exit when the requested class is already active.
    if LAST_PRIORITY_CLASS.with(|last| last.get()) == Some(priority) {
        return;
    }

    unsafe {
        let thread = GetCurrentThread();

        THREAD_BG_MODE_ACTIVE.with(|bg_active| match priority {
            IOPriority::Interactive => {
                if bg_active.replace(false) {
                    let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_END);
                }
                let _ = SetThreadPriority(thread, THREAD_PRIORITY_ABOVE_NORMAL);
            }
            IOPriority::Prefetch => {
                if bg_active.replace(false) {
                    let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_END);
                }
                let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
            }
            IOPriority::Background => {
                if !bg_active.get() {
                    let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_BEGIN);
                    bg_active.set(true);
                }
                let _ = SetThreadPriority(thread, THREAD_PRIORITY_LOWEST);
            }
        });
    }

    LAST_PRIORITY_CLASS.with(|last| last.set(Some(priority)));
}

/// Reset thread priority to normal.
pub(super) fn reset_thread_priority() {
    use windows::Win32::System::Threading::*;

    unsafe {
        let thread = GetCurrentThread();

        THREAD_BG_MODE_ACTIVE.with(|bg_active| {
            if bg_active.replace(false) {
                let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_END);
            }
        });

        let _ = SetThreadPriority(thread, THREAD_PRIORITY_NORMAL);
    }

    LAST_PRIORITY_CLASS.with(|last| last.set(None));
}
