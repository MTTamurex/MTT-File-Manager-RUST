use super::notifications::Registration;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

static RETIRED: Mutex<Vec<Registration>> = Mutex::new(Vec::new());

/// Navigation may discard a watcher during query-remove. Retain its PnP
/// registration until the outcome so query-remove-failed can resume the drive.
/// Unregister on this worker, never inside the Windows callback.
pub(super) fn retain_until_outcome(registration: Registration) {
    RETIRED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(registration);
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_millis(250));
            let ready = {
                let mut registrations = RETIRED.lock().unwrap_or_else(|error| error.into_inner());
                let mut ready = Vec::new();
                let mut index = 0;
                while index < registrations.len() {
                    if registrations[index].can_release() {
                        ready.push(registrations.swap_remove(index));
                    } else {
                        index += 1;
                    }
                }
                ready
            };
            drop(ready);
        });
    });
}
