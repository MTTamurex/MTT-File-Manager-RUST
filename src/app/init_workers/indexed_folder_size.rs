use crate::infrastructure::drive_indexing::SharedDriveIndexing;
use mtt_search_protocol::DRIVE_INDEXING_DISABLED_ERROR;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(super) fn is_retryable_folder_size_service_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("sizes not loaded")
        || lower.contains("volume not ready")
        || lower.contains("volume not indexed")
        || lower.contains("index refresh pending")
        || lower.contains("live path validation failed")
        || lower.contains("all pipe instances are busy")
        || lower.contains("no process is on the other end of the pipe")
        || lower.contains("pipe closed during read")
        || lower.contains("search service timeout")
        || lower.contains("peeknamedpipe failed")
        || lower.contains("readfile failed")
        || lower.contains("writefile failed")
}

pub(super) fn query(
    path: &Path,
    cancel: &Arc<AtomicBool>,
    policy: &SharedDriveIndexing,
) -> Result<(u64, u64, u64), String> {
    query_with(
        path,
        cancel,
        policy,
        crate::infrastructure::global_search::folder_size,
    )
}

fn query_with(
    path: &Path,
    cancel: &AtomicBool,
    policy: &SharedDriveIndexing,
    mut fetch: impl FnMut(&Path) -> Result<(u64, u64, u64), String>,
) -> Result<(u64, u64, u64), String> {
    let mut diagnostic = super::folder_size_diagnostics::Query::new();
    let generation = policy.generation();
    let check = || {
        if cancel.load(Ordering::Acquire) {
            Err("cancelled".to_string())
        } else if !policy.is_path_enabled(path) {
            Err(DRIVE_INDEXING_DISABLED_ERROR.to_string())
        } else if generation != policy.generation() {
            Err("cancelled".to_string())
        } else {
            Ok(())
        }
    };
    let mut last_error = String::from("Search service not available");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut retry_delay = Duration::from_millis(50);
    for attempt in 0..20 {
        check()?;
        if Instant::now() >= deadline {
            diagnostic.outcome("deadline");
            break;
        }
        diagnostic.attempt();
        match fetch(path) {
            Ok(result) => {
                check()?;
                diagnostic.outcome("completed");
                return Ok(result);
            }
            Err(error) => {
                let retryable = is_retryable_folder_size_service_error(&error);
                last_error = error;
                if !retryable || attempt + 1 >= 20 {
                    break;
                }
                let sleep_end = Instant::now() + retry_delay;
                while Instant::now() < sleep_end {
                    check()?;
                    std::thread::sleep(
                        sleep_end
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(50)),
                    );
                }
                retry_delay = (retry_delay * 2).min(Duration::from_secs(2));
            }
        }
    }
    Err(last_error)
}

#[cfg(test)]
mod tests;
