//! Tracks standalone viewer subprocesses spawned by the main app.

use std::process::Child;
use std::sync::{Mutex, OnceLock};

fn child_processes() -> &'static Mutex<Vec<Child>> {
    static CHILD_PROCESSES: OnceLock<Mutex<Vec<Child>>> = OnceLock::new();
    CHILD_PROCESSES.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register(child: Child) {
    let Ok(mut children) = child_processes().lock() else {
        log::warn!("[VIEWER-PROCESS] Failed to track spawned viewer process");
        return;
    };

    children.push(child);
}

pub fn reap_exited() {
    let Ok(mut children) = child_processes().lock() else {
        return;
    };

    children.retain_mut(|child| match child.try_wait() {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(error) => {
            log::warn!(
                "[VIEWER-PROCESS] Failed to query viewer subprocess pid={}: {}",
                child.id(),
                error
            );
            false
        }
    });
}

pub fn terminate_all() {
    let Ok(mut children) = child_processes().lock() else {
        log::warn!("[VIEWER-PROCESS] Failed to lock viewer process registry during shutdown");
        return;
    };

    for mut child in children.drain(..) {
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                log::debug!(
                    "[VIEWER-PROCESS] Terminating viewer subprocess pid={}",
                    child.id()
                );
                if let Err(error) = child.kill() {
                    log::warn!(
                        "[VIEWER-PROCESS] Failed to terminate viewer subprocess pid={}: {}",
                        child.id(),
                        error
                    );
                }
            }
            Err(error) => {
                log::warn!(
                    "[VIEWER-PROCESS] Failed to query viewer subprocess pid={}: {}",
                    child.id(),
                    error
                );
            }
        }
    }
}
