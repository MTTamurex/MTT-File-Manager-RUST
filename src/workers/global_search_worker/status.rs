use super::*;
use mtt_search_protocol::IndexStatusInfo;

pub(super) fn refresh_and_send_status(
    sender: &Sender<GlobalSearchResponse>,
    session_index: &mut crate::infrastructure::user_session_search::UserSessionSearchIndex,
    last_known_available: &mut bool,
    last_known_total_indexed: &mut u64,
    last_known_status_volumes: &mut Vec<VolumeStatus>,
    last_known_service_executable_path: &mut String,
    consecutive_failures: &mut u8,
) {
    let ping_ok = crate::infrastructure::global_search::ping();
    let mut status_ok: Option<IndexStatusInfo> = None;
    let mut last_error: Option<String> = None;
    if ping_ok {
        for attempt in 0..STATUS_RETRY_COUNT {
            match crate::infrastructure::global_search::get_status() {
                Ok(status) => {
                    status_ok = Some(status);
                    break;
                }
                Err(error) => {
                    last_error = Some(error.clone());
                    if attempt + 1 < STATUS_RETRY_COUNT && is_transient_ipc_error(&error) {
                        std::thread::sleep(std::time::Duration::from_millis(140));
                        continue;
                    }
                    break;
                }
            }
        }
        if let Some(status) = status_ok {
            if *last_known_service_executable_path != status.service_executable_path {
                log::info!(
                    "[GLOBAL-SEARCH] Connected to service binary: {}",
                    status.service_executable_path
                );
                *last_known_service_executable_path = status.service_executable_path;
            }
            *last_known_available = true;
            *last_known_total_indexed = status.total_files_indexed;
            *last_known_status_volumes = status.volumes;
            *consecutive_failures = 0;
        } else {
            *consecutive_failures = (*consecutive_failures).saturating_add(1);
            let transient = last_error.as_deref().is_some_and(is_transient_ipc_error);
            if !(transient && *last_known_available)
                && *consecutive_failures >= OFFLINE_FAILURE_THRESHOLD
            {
                *last_known_available = false;
            }
            if let Some(error) = last_error {
                log::warn!("[GLOBAL-SEARCH] Service status refresh failed consecutive_failures={} reason={}", *consecutive_failures, error);
            }
        }
    } else {
        *consecutive_failures = (*consecutive_failures).saturating_add(1);
        if *consecutive_failures >= OFFLINE_FAILURE_THRESHOLD {
            *last_known_available = false;
        }
    }
    session_index.refresh(
        &service_volume_letters(last_known_status_volumes),
        *last_known_available,
        false,
    );
    let _ = sender.send(GlobalSearchResponse::Status {
        service_available: *last_known_available,
        available: *last_known_available || session_index.has_indexed_items(),
        total_indexed: *last_known_total_indexed,
        session_total_indexed: session_index.total_indexed(),
        indexing_in_progress: last_known_status_volumes
            .iter()
            .any(volume_is_actively_scanning),
        volumes: last_known_status_volumes.clone(),
    });
}
