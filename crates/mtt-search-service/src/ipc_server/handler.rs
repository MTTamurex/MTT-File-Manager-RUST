use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

use windows::Win32::Foundation::HANDLE;

use crate::file_index::{IndexState, VolumeIndex};
use crate::indexing_progress::IndexingProgress;
use crate::ipc_authorization::{
    collect_authorized_search_page,
    current_client_can_read_path, PipeImpersonationGuard,
};
use crate::index_db;
use crate::security_policy::IpcSecurityPolicy;
use crate::FtsState;
use mtt_search_protocol::*;

use super::pipe_io::{read_message, send_response};
use super::{MAX_QUERY_OFFSET, MAX_QUERY_RESULTS};

/// Minimum seconds between WarmIndex operations to prevent DoS via repeated warm requests.
const WARM_COOLDOWN_SECS: u64 = 60;

pub(super) fn handle_client(
    pipe: HANDLE,
    indices: &Arc<RwLock<Vec<VolumeIndex>>>,
    indexing_progress: &Arc<IndexingProgress>,
    is_warming: &Arc<AtomicBool>,
    last_warm_epoch_secs: &Arc<AtomicU64>,
    security_policy: &IpcSecurityPolicy,
    _fts_searcher: &Option<Arc<index_db::FtsSearcher>>,
    _fts_state: &Arc<FtsState>,
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
                std::thread::spawn(move || {
                    eprintln!("[IPC] WarmIndex: warming in-memory index...");
                    let start = std::time::Instant::now();
                    {
                        let lock = indices_clone.read();
                        let mut touched = 0u64;
                        for vol in lock.iter() {
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
                    warming_flag.store(false, Ordering::SeqCst);
                });
            }
        }
        SearchRequest::GetStatus => {
            let indices_lock = indices.read();
            let status = build_status_response(&indices_lock, indexing_progress, security_policy);

            let _ = send_response(
                pipe,
                &SearchResponse::Status(status),
            );
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
                    let _ = send_response(pipe, &SearchResponse::Error("Authorization failed".to_string()));
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
                let indices_lock = indices.read();
                paths
                    .iter()
                    .filter_map(|path_str| {
                        let drive_letter = match path_str.chars().next() {
                            Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
                            _ => return None,
                        };
                        let vol = indices_lock
                            .iter()
                            .find(|v| v.drive_letter == drive_letter)?;
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
            }; // indices_lock released here

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
            let drive_letter = match path.chars().next() {
                Some(c) if c.is_ascii_alphabetic() => c.to_ascii_uppercase(),
                _ => {
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error("Invalid path".to_string()),
                    );
                    return;
                }
            };

            // Compute folder size from in-memory index.
            let result = {
                let indices_lock = indices.read();
                let vol = match indices_lock.iter().find(|v| v.drive_letter == drive_letter) {
                    Some(v) => v,
                    None => {
                        drop(indices_lock);
                        let _ = send_response(
                            pipe,
                            &SearchResponse::Error("Volume not indexed".to_string()),
                        );
                        return;
                    }
                };
                if !matches!(vol.state, IndexState::Ready) {
                    drop(indices_lock);
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error("Volume not ready".to_string()),
                    );
                    return;
                }
                if !vol.sizes_loaded {
                    drop(indices_lock);
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error("Sizes not loaded".to_string()),
                    );
                    return;
                }
                match vol.resolve_path_to_frn(&path) {
                    Some(frn) => Ok(crate::mft_reader::folder_size_for_service(vol, frn)),
                    None => Err("Path not found in index"),
                }
            };

            match result {
                Ok((total_size, file_count)) => {
                    eprintln!(
                        "[FOLDER-SIZE] responding path={} total_gb={:.2} files={}",
                        crate::redact_paths(&path),
                        total_size as f64 / 1_073_741_824.0,
                        file_count,
                    );
                    let _ = send_response(
                        pipe,
                        &SearchResponse::FolderSize {
                            path,
                            total_size,
                            file_count,
                        },
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[FOLDER-SIZE] error path={} reason={}",
                        crate::redact_paths(&path),
                        e,
                    );
                    let _ = send_response(
                        pipe,
                        &SearchResponse::Error(e.to_string()),
                    );
                }
            }
        }
    }
}

fn build_status_response(
    indices: &[VolumeIndex],
    indexing_progress: &IndexingProgress,
    security_policy: &IpcSecurityPolicy,
) -> IndexStatusInfo {
    let mut volume_map = BTreeMap::<char, VolumeStatus>::new();
    let service_executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
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
        for vol in indices {
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
        for vol in indices {
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
                        eprintln!("[IPC] Volume {} status error (redacted from client): {}", vol.drive_letter, e);
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
                sizes_loading: matches!(vol.state, crate::file_index::IndexState::Ready) && !vol.sizes_loaded,
            };

            match volume_map.get_mut(&vol.drive_letter) {
                Some(existing)
                    if existing.state == "scanning"
                        && next_status.state == "ready" =>
                {
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
    use crate::file_index::IndexState;

    fn make_volume(drive_letter: char, records_len: usize, state: IndexState) -> VolumeIndex {
        let mut index = VolumeIndex::new(drive_letter);
        index.state = state;
        for i in 0..records_len {
            index.insert_record(i as u64 + 1, "sample.txt", 0, false, false);
        }
        index
    }

    #[test]
    fn status_response_keeps_existing_behavior_by_default() {
        let indices = vec![
            make_volume('C', 2, IndexState::Ready),
            make_volume('D', 1, IndexState::Scanning),
        ];
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
        let indices = vec![
            make_volume('C', 2, IndexState::Ready),
            make_volume('D', 1, IndexState::Error("io".to_string())),
        ];
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
        let indices = vec![make_volume('C', 2, IndexState::Ready)];
        let progress = IndexingProgress::new();
        let policy = IpcSecurityPolicy {
            redact_status_metrics: false,
        };

        progress.set_scanning('D', 7);

        let status = build_status_response(&indices, &progress, &policy);
        assert_eq!(status.total_files_indexed, 9);
        assert_eq!(status.volumes.len(), 2);
        assert!(status
            .volumes
            .iter()
            .any(|volume| volume.drive_letter == 'D' && volume.state == "scanning" && volume.files_indexed == 7));
    }
}
