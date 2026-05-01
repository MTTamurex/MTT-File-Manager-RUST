use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use windows::Win32::Foundation::HANDLE;

use crate::file_index::IndexState;
use crate::indexing_progress::IndexingProgress;
use crate::ipc_authorization::{
    collect_authorized_search_page, current_client_can_read_path, trusted_file_manager_client,
    PipeImpersonationGuard,
};
use crate::security_policy::IpcSecurityPolicy;
use crate::volume_indices::{self, SharedVolumeIndices, VolumeIndexHandle};
use mtt_search_protocol::*;

use super::pipe_io::{read_message, send_response};
use super::{MAX_QUERY_OFFSET, MAX_QUERY_RESULTS};

/// Minimum seconds between WarmIndex operations to prevent DoS via repeated warm requests.
const WARM_COOLDOWN_SECS: u64 = 60;
const ZERO_SIZE_FOLDER_REPAIR_LIMIT: usize = 4_096;

fn repair_suspicious_zero_folder_size(
    handle: &VolumeIndexHandle,
    drive_letter: char,
    path: &str,
    dir_frn: u64,
    summary: (u64, u64, u64),
) -> (u64, u64, u64) {
    let (total_size, file_count, folder_count) = summary;
    if total_size > 0 || file_count == 0 {
        return summary;
    }

    let volume = match crate::usn_journal::open_volume(drive_letter) {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!(
                "[FOLDER-SIZE] zero-size repair open-volume failed path={} reason={}",
                crate::redact_paths(path),
                crate::redact_paths(&error),
            );
            return summary;
        }
    };

    let record_size = match crate::mft_reader::query_mft_geometry_pub(volume) {
        Ok(record_size) => record_size,
        Err(error) => {
            crate::usn_journal::close_volume(volume);
            eprintln!(
                "[FOLDER-SIZE] zero-size repair geometry failed path={} reason={}",
                crate::redact_paths(path),
                crate::redact_paths(&error),
            );
            return summary;
        }
    };

    let (candidate_count, repaired_count, refreshed_summary) = {
        let mut vol = handle.write();
        let candidates =
            vol.collect_zero_size_file_frns_in_subtree(dir_frn, ZERO_SIZE_FOLDER_REPAIR_LIMIT);
        if candidates.is_empty() {
            (0usize, 0usize, summary)
        } else {
            let repaired = crate::mft_reader::repair_zero_size_file_frns(
                volume,
                &mut vol,
                &candidates,
                record_size,
            );
            let refreshed = crate::mft_reader::folder_size_for_service(&vol, dir_frn);
            (candidates.len(), repaired, refreshed)
        }
    };

    crate::usn_journal::close_volume(volume);

    if repaired_count > 0 {
        eprintln!(
            "[FOLDER-SIZE] repaired-zero-sizes path={} candidates={} repaired={} total_gb={:.2} files={} folders={}",
            crate::redact_paths(path),
            candidate_count,
            repaired_count,
            refreshed_summary.0 as f64 / 1_073_741_824.0,
            refreshed_summary.1,
            refreshed_summary.2,
        );
        return refreshed_summary;
    }

    if candidate_count > 0 {
        eprintln!(
            "[FOLDER-SIZE] zero-size repair no-change path={} candidates={} total_gb={:.2} files={} folders={}",
            crate::redact_paths(path),
            candidate_count,
            total_size as f64 / 1_073_741_824.0,
            file_count,
            folder_count,
        );
    }

    summary
}

pub(super) fn handle_client(
    pipe: HANDLE,
    indices: &SharedVolumeIndices,
    indexing_progress: &Arc<IndexingProgress>,
    is_warming: &Arc<AtomicBool>,
    last_warm_epoch_secs: &Arc<AtomicU64>,
    security_policy: &IpcSecurityPolicy,
) {
    let request_data = match read_message(pipe) {
        Some(data) => data,
        None => return,
    };

    let request: SearchRequest = match decode_message(&request_data) {
        Ok(r) => r,
        Err(e) => {
            // Log the real error internally, send generic message to client
            eprintln!("[IPC] Failed to decode request: {}", e);
            let _ = send_response(pipe, &SearchResponse::Error("Invalid request".to_string()));
            return;
        }
    };

    if let Err(e) = request.validate() {
        eprintln!("[IPC] Request validation failed: {}", e);
        let _ = send_response(pipe, &SearchResponse::Error("Invalid request".to_string()));
        return;
    }

    match request {
        SearchRequest::Ping => {
            let _ = send_response(pipe, &SearchResponse::Pong);
        }
        SearchRequest::WarmIndex => {
            if !require_trusted_metadata_client(pipe, "WarmIndex") {
                return;
            }

            // Respond immediately so the client is not blocked.
            let _ = send_response(pipe, &SearchResponse::WarmStarted);

            // SEC: Cooldown to prevent DoS via repeated WarmIndex requests.
            // If a warm completed within the last 60 seconds, skip.
            let now_epoch = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let last_warm = last_warm_epoch_secs.load(Ordering::Relaxed);
            if now_epoch.saturating_sub(last_warm) < WARM_COOLDOWN_SECS {
                return;
            }

            // Only spawn the warming thread if one is not already running.
            if is_warming
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed)
                .is_ok()
            {
                let indices_clone = indices.clone();
                let warming_flag = is_warming.clone();
                let warm_epoch = last_warm_epoch_secs.clone();
                let spawn_result =
                    std::thread::Builder::new()
                        .name("warm-index".into())
                        .spawn(move || {
                            struct WarmGuard(Arc<AtomicBool>);
                            impl Drop for WarmGuard {
                                fn drop(&mut self) {
                                    self.0.store(false, Ordering::SeqCst);
                                }
                            }

                            let _guard = WarmGuard(Arc::clone(&warming_flag));

                            eprintln!("[IPC] WarmIndex: warming in-memory index...");
                            let start = std::time::Instant::now();
                            {
                                let handles = volume_indices::snapshot_handles(&indices_clone);
                                let mut touched = 0u64;
                                for handle in &handles {
                                    let vol = handle.read();
                                    let arena_bytes = vol.names.as_bytes();
                                    for chunk in arena_bytes.chunks(4096) {
                                        black_box(&chunk[0]);
                                    }
                                    touched += vol.records.len() as u64;
                                }
                                eprintln!(
                                    "[IPC] WarmIndex: touched {} records in {:.2}s",
                                    touched,
                                    start.elapsed().as_secs_f64()
                                );
                            }
                            // Record completion timestamp for cooldown.
                            let done_epoch = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            warm_epoch.store(done_epoch, Ordering::Relaxed);
                        });

                if let Err(error) = spawn_result {
                    is_warming.store(false, Ordering::SeqCst);
                    eprintln!("[IPC] WarmIndex spawn failed: {}", error);
                }
            }
        }
        SearchRequest::GetStatus => {
            if !require_trusted_metadata_client(pipe, "GetStatus") {
                return;
            }

            let handles = volume_indices::snapshot_handles(indices);
            let status = build_status_response(&handles, indexing_progress, security_policy);

            let _ = send_response(pipe, &SearchResponse::Status(status));
        }
        SearchRequest::Query {
            text,
            offset,
            limit,
        } => {
            // Input validation: cap offset/limit and text length.
            let offset = (offset as usize).min(MAX_QUERY_OFFSET);
            let limit = (limit as usize).min(MAX_QUERY_RESULTS);

            let text = if text.len() > MAX_QUERY_TEXT_LEN {
                text[..text.floor_char_boundary(MAX_QUERY_TEXT_LEN)].to_string()
            } else {
                text
            };

            if text.is_empty() {
                let _ = send_response(
                    pipe,
                    &SearchResponse::Results {
                        items: Vec::new(),
                        has_more: false,
                        total_matches: Some(0),
                    },
                );
                return;
            }

            // Always use in-memory SIMD search (Phase 3). The lowered NameArena
            // + memchr::memmem is faster than FTS5 for all query lengths, and
            // has zero index build overhead.
            let result = collect_authorized_search_page(pipe, indices, &text, offset, limit);

            match result {
                Ok(page) => {
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Results {
                            items: page.items,
                            has_more: page.has_more,
                            total_matches: page.total_matches,
                        },
                    );
                }
                Err(e) => {
                    if e == "Client disconnected" {
                        return;
                    }
                    eprintln!("[IPC] Authorization check failed: {}", e);
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error("Authorization failed".to_string()),
                    );
                }
            }
        }
        SearchRequest::CheckPathsModified {
            paths,
            threshold_secs,
        } => {
            // SEC: Impersonate the connecting client before checking any paths.
            // Without this, any local user could probe modification times of
            // directories they have no read access to (information disclosure).
            let _guard = match PipeImpersonationGuard::new(pipe) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("[IPC] CheckPathsModified impersonation failed: {}", e);
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error("Authorization failed".to_string()),
                    );
                    return;
                }
            };

            let threshold_dur = std::time::Duration::from_secs(threshold_secs as u64);
            let now = std::time::Instant::now();

            // ── Snapshot modification times under lock, then release before
            // authorization ── CreateFileW (inside current_client_can_read_path)
            // can block on AV, cloud filters, or FUSE mounts. Holding the
            // RwLock during those calls would block index writers (USN journal
            // incremental updates). Instead we snapshot the cheap index lookups
            // under a short lock and defer the slow I/O authorization to after
            // the lock is released — the same pattern used by
            // collect_authorized_search_page.
            let candidates: Vec<(String, bool)> = {
                paths
                    .iter()
                    .filter_map(|path_str| {
                        let drive_letter = match path_str.chars().next() {
                            Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
                            _ => return None,
                        };
                        let handle = volume_indices::find_handle(indices, drive_letter)?;
                        let vol = handle.read();
                        if !matches!(vol.state, IndexState::Ready) {
                            return None;
                        }
                        let frn = vol.resolve_path_to_frn(path_str)?;
                        let &modified_at = vol.dir_modified_at.get(&frn)?;
                        let age = now
                            .checked_duration_since(modified_at)
                            .unwrap_or(std::time::Duration::MAX);
                        Some((path_str.clone(), age <= threshold_dur))
                    })
                    .collect()
            }; // per-volume read locks released here

            // SEC: Verify the client has read access to each candidate path
            // before revealing whether it was recently modified. This runs
            // WITHOUT the index lock held.
            let modified: Vec<String> = candidates
                .into_iter()
                .filter(|(path_str, recently_modified)| {
                    *recently_modified && current_client_can_read_path(path_str)
                })
                .map(|(path_str, _)| path_str)
                .collect();

            let _ = send_response(pipe, &SearchResponse::PathsModified { modified });
        }
        SearchRequest::FolderSize { path } => {
            if !require_trusted_metadata_client(pipe, "FolderSize") {
                return;
            }

            let drive_letter = match path.chars().next() {
                Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
                _ => {
                    let _ = send_response(pipe, &SearchResponse::Error("Invalid path".to_string()));
                    return;
                }
            };

            // NOTE: We intentionally do NOT impersonate the client and gate
            // FolderSize on `current_client_can_read_path(&path)`. By the time
            // the app issues a FolderSize request the user is already viewing
            // the parent listing in the UI (and therefore knows the folder
            // exists), so the size value carries no additional disclosure
            // risk worth blocking core functionality for. An impersonated
            // CreateFileW(GENERIC_READ) gate also produced false negatives on
            // legitimately readable system folders (e.g. C:\PerfLogs) due to
            // named-pipe SQOS / impersonation-level interactions, breaking
            // size aggregation for ordinary users. See git log for the
            // original CRIT-2 reasoning that this comment supersedes.

            // Compute folder size from in-memory index.
            let handle = match volume_indices::find_handle(indices, drive_letter) {
                Some(h) => h,
                None => {
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error("Volume not indexed".to_string()),
                    );
                    return;
                }
            };

            let result = {
                let vol = handle.read();
                if !matches!(vol.state, IndexState::Ready) {
                    drop(vol);
                    let _ =
                        send_response(pipe, &SearchResponse::Error("Volume not ready".to_string()));
                    return;
                }
                if !vol.sizes_loaded {
                    drop(vol);
                    let _ =
                        send_response(pipe, &SearchResponse::Error("Sizes not loaded".to_string()));
                    return;
                }
                match vol.resolve_path_to_frn(&path) {
                    Some(frn) => Ok((frn, crate::mft_reader::folder_size_for_service(&vol, frn))),
                    None => Err("Path not found in index"),
                }
            };

            match result {
                Ok((dir_frn, summary)) => {
                    let (total_size, file_count, folder_count) = repair_suspicious_zero_folder_size(
                        &handle,
                        drive_letter,
                        &path,
                        dir_frn,
                        summary,
                    );
                    eprintln!(
                        "[FOLDER-SIZE] responding path={} total_gb={:.2} files={} folders={}",
                        crate::redact_paths(&path),
                        total_size as f64 / 1_073_741_824.0,
                        file_count,
                        folder_count,
                    );
                    let _ = send_response(
                        pipe,
                        &SearchResponse::FolderSize {
                            path,
                            total_size,
                            file_count,
                            folder_count,
                        },
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[FOLDER-SIZE] error path={} reason={}",
                        crate::redact_paths(&path),
                        e,
                    );
                    let _ = send_response(pipe, &SearchResponse::Error(e.to_string()));
                }
            }
        }
    }
}

fn require_trusted_metadata_client(pipe: HANDLE, operation: &str) -> bool {
    match trusted_file_manager_client(pipe) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "[IPC] {} rejected unauthorized client: {}",
                operation,
                crate::redact_paths(&error),
            );
            let _ = send_response(
                pipe,
                &SearchResponse::Error("Authorization failed".to_string()),
            );
            false
        }
    }
}

fn build_status_response(
    handles: &[VolumeIndexHandle],
    indexing_progress: &IndexingProgress,
    security_policy: &IpcSecurityPolicy,
) -> IndexStatusInfo {
    let mut volume_map = BTreeMap::<char, VolumeStatus>::new();
    // SEC: Return only the executable basename, never the full path. Exposing
    // the install directory to any pipe client leaks layout details that aid
    // PATH/DLL hijacking and reconnaissance.
    let service_executable_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "<unknown>".to_string());
    let progress_snapshot = indexing_progress.snapshot();

    if security_policy.redact_status_metrics {
        for progress in progress_snapshot {
            volume_map.insert(
                progress.drive_letter,
                VolumeStatus {
                    drive_letter: progress.drive_letter,
                    state: "redacted".to_string(),
                    files_indexed: 0,
                    phase: "redacted".to_string(),
                    phase_progress: None,
                    phase_total: None,
                    sizes_loading: false,
                },
            );
        }
        for handle in handles {
            let vol = handle.read();
            volume_map.insert(
                vol.drive_letter,
                VolumeStatus {
                    drive_letter: vol.drive_letter,
                    state: "redacted".to_string(),
                    files_indexed: 0,
                    phase: "redacted".to_string(),
                    phase_progress: None,
                    phase_total: None,
                    sizes_loading: false,
                },
            );
        }
    } else {
        for progress in progress_snapshot {
            volume_map.insert(progress.drive_letter, progress);
        }
        for handle in handles {
            let vol = handle.read();
            let count = vol.records.len() as u64;
            let next_status = VolumeStatus {
                drive_letter: vol.drive_letter,
                state: match &vol.state {
                    IndexState::NotStarted => "not_started".to_string(),
                    IndexState::Scanning => "scanning".to_string(),
                    IndexState::Ready => "ready".to_string(),
                    // SEC: Redact internal error details to prevent information leakage
                    // (filesystem paths, driver names, OS error codes). Log the real
                    // error server-side for diagnostics.
                    IndexState::Error(e) => {
                        eprintln!(
                            "[IPC] Volume {} status error (redacted from client): {}",
                            vol.drive_letter, e
                        );
                        "error".to_string()
                    }
                },
                files_indexed: count,
                phase: match &vol.state {
                    IndexState::NotStarted => "not_started".to_string(),
                    IndexState::Scanning => "scanning".to_string(),
                    IndexState::Ready => "ready".to_string(),
                    IndexState::Error(_) => "error".to_string(),
                },
                phase_progress: None,
                phase_total: None,
                sizes_loading: matches!(vol.state, crate::file_index::IndexState::Ready)
                    && !vol.sizes_loaded,
            };

            match volume_map.get_mut(&vol.drive_letter) {
                Some(existing) if existing.state == "scanning" && next_status.state == "ready" => {
                    existing.files_indexed = existing.files_indexed.max(next_status.files_indexed);
                }
                _ => {
                    volume_map.insert(vol.drive_letter, next_status);
                }
            }
        }
    }

    let volumes = volume_map.into_values().collect::<Vec<_>>();
    let total_indexed = volumes.iter().map(|volume| volume.files_indexed).sum();

    IndexStatusInfo {
        volumes,
        total_files_indexed: total_indexed,
        service_executable_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::{IndexState, VolumeIndex};
    use crate::volume_indices::handle_from;

    fn make_volume(drive_letter: char, records_len: usize, state: IndexState) -> VolumeIndex {
        let mut index = VolumeIndex::empty(drive_letter);
        index.state = state;
        for i in 0..records_len {
            index.insert_record(i as u64 + 1, "sample.txt", 0, false, false);
        }
        index
    }

    fn make_handles(volumes: Vec<VolumeIndex>) -> Vec<VolumeIndexHandle> {
        volumes.into_iter().map(handle_from).collect()
    }

    #[test]
    fn status_response_keeps_existing_behavior_by_default() {
        let indices = make_handles(vec![
            make_volume('C', 2, IndexState::Ready),
            make_volume('D', 1, IndexState::Scanning),
        ]);
        let progress = IndexingProgress::new();
        let policy = IpcSecurityPolicy {
            redact_status_metrics: false,
        };

        let status = build_status_response(&indices, &progress, &policy);
        assert_eq!(status.total_files_indexed, 3);
        assert_eq!(status.volumes.len(), 2);
        assert_eq!(status.volumes[0].files_indexed, 2);
        assert_eq!(status.volumes[1].state, "scanning");
    }

    #[test]
    fn status_response_redacts_when_policy_enabled() {
        let indices = make_handles(vec![
            make_volume('C', 2, IndexState::Ready),
            make_volume('D', 1, IndexState::Error("io".to_string())),
        ]);
        let progress = IndexingProgress::new();
        let policy = IpcSecurityPolicy {
            redact_status_metrics: true,
        };

        let status = build_status_response(&indices, &progress, &policy);
        assert_eq!(status.total_files_indexed, 0);
        assert_eq!(status.volumes.len(), 2);
        assert_eq!(status.volumes[0].state, "redacted");
        assert_eq!(status.volumes[0].files_indexed, 0);
        assert_eq!(status.volumes[1].state, "redacted");
    }

    #[test]
    fn status_response_includes_inflight_volume_progress() {
        let indices = make_handles(vec![make_volume('C', 2, IndexState::Ready)]);
        let progress = IndexingProgress::new();
        let policy = IpcSecurityPolicy {
            redact_status_metrics: false,
        };

        progress.set_scanning('D', 7, "filesystem_scan");

        let status = build_status_response(&indices, &progress, &policy);
        assert_eq!(status.total_files_indexed, 9);
        assert_eq!(status.volumes.len(), 2);
        assert!(status
            .volumes
            .iter()
            .any(|volume| volume.drive_letter == 'D'
                && volume.state == "scanning"
                && volume.files_indexed == 7));
    }
}
