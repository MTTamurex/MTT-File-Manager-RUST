//! The Shell discovers packaged ExplorerCommand registrations asynchronously on
//! the first query in a fresh host. An existing HMENU is not updated when that
//! discovery finishes. Re-query once before publishing the first snapshot.

use super::{extract_context_menu, IContextMenu, Result, ShellMenuContext};
use std::cell::Cell;
use std::time::{Duration, Instant};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, PostQuitMessage, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
};

thread_local! {
    static INITIAL_QUERY_COMPLETE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn extract(
    mut create_context: impl FnMut() -> Result<IContextMenu>,
) -> Result<ShellMenuContext> {
    let initial = extract_context_menu(create_context()?)?;
    if INITIAL_QUERY_COMPLETE.with(Cell::get) {
        return Ok(initial);
    }

    // This runs only on the Shell STA worker, while the UI retains its loading
    // placeholder. Bound the cold-start wait; never retry every menu or special-
    // case an application name. Subsequent queries have no added wait.
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                if message.message == WM_QUIT {
                    PostQuitMessage(message.wParam.0 as i32);
                    return Ok(initial);
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
                if Instant::now() >= deadline {
                    break;
                }
            }
        }
        std::thread::sleep(
            Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
        );
    }

    // Keep only the final HMENU and its command IDs. Combining snapshots could
    // invoke a different command after packaged extensions shift the ID range.
    drop(initial);
    let complete = extract_context_menu(create_context()?)?;
    INITIAL_QUERY_COMPLETE.with(|ready| ready.set(true));
    Ok(complete)
}
