use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::RwLock;

use windows::Win32::Foundation::HANDLE;

use crate::file_index::{IndexState, VolumeIndex};
use crate::ipc_authorization::{
    collect_authorized_fts_page, collect_authorized_search_page,
    current_client_can_read_path, PipeImpersonationGuard,
};
use crate::index_db;
use crate::security_policy::IpcSecurityPolicy;
use mtt_search_protocol::*;

use super::pipe_io::{read_message, send_response};
use super::{MAX_QUERY_OFFSET, MAX_QUERY_RESULTS};

/// Minimum seconds between WarmIndex operations to prevent DoS via repeated warm requests.
const WARM_COOLDOWN_SECS: u64 = 60;

pub(super) fn handle_client(
    pipe: HANDLE,
    indices: &Arc<RwLock<Vec<VolumeIndex>>>,
    is_warming: &Arc<AtomicBool>,
    last_warm_epoch_secs: &Arc<AtomicU64>,
    security_policy: &IpcSecurityPolicy,
    fts_searcher: &Option<Arc<index_db::FtsSearcher>>,
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
            let status = build_status_response(&indices_lock, security_policy);

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

            // FTS5 path: use trigram-indexed search when all tokens are ≥3 chars.
            // Shorter tokens would cause FTS5 to fall back to a full table scan
            // (slower than the in-memory linear scan), so we keep the old path.
            let min_token_len = text
                .split_whitespace()
                .map(|t| t.len())
                .min()
                .unwrap_or(0);
            let use_fts = min_token_len >= 3 && fts_searcher.is_some();

            // ── Snapshot candidates under lock, then release before authorization ──
            // Authorization calls CreateFileW which can be slow (AV, cloud, FUSE).
            // Holding the RwLock during those calls would block index writers.
            let result = if use_fts {
                // FTS path: query SQLite (no lock needed), then resolve paths
                // from the in-memory index under a short lock.
                match collect_authorized_fts_page(
                    pipe,
                    fts_searcher.as_ref().unwrap(),
                    indices,
                    &text,
                    offset,
                    limit,
                ) {
                    Ok(page) => Ok(page),
                    Err(e) => {
                        // Only fall back to linear scan for structural FTS errors
                        // (corrupt index, schema mismatch).  Transient problems
                        // like client disconnects or lock contention would hit the
                        // linear path just as hard (or worse — it holds the read
                        // lock much longer), so propagate those directly.
                        let is_transient = e.contains("Client disconnected")
                            || e.contains("busy")
                            || e.contains("timeout")
                            || e.contains("locked");
                        if is_transient {
                            eprintln!(
                                "[IPC] FTS query transient error for '{}', skipping linear fallback: {}",
                                text,
                                e
                            );
                            Err(e)
                        } else {
                            eprintln!(
                                "[IPC] FTS query failed for '{}', falling back to linear scan: {}",
                                text,
                                e
                            );
                            collect_authorized_search_page(pipe, indices, &text, offset, limit)
                        }
                    }
                }
            } else {
                collect_authorized_search_page(pipe, indices, &text, offset, limit)
            };

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
    }
}

fn build_status_response(indices: &[VolumeIndex], security_policy: &IpcSecurityPolicy) -> IndexStatusInfo {
    let mut total_indexed = 0u64;
    let mut volumes = Vec::with_capacity(indices.len());

    if security_policy.redact_status_metrics {
        for vol in indices {
            volumes.push(VolumeStatus {
                drive_letter: vol.drive_letter,
                state: "redacted".to_string(),
                files_indexed: 0,
            });
        }
    } else {
        for vol in indices {
            let count = vol.records.len() as u64;
            total_indexed += count;
            volumes.push(VolumeStatus {
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
            });
        }
    }

    IndexStatusInfo {
        volumes,
        total_files_indexed: total_indexed,
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
            index.insert_record(i as u64 + 1, "sample.txt", 0, false);
        }
        index
    }

    #[test]
    fn status_response_keeps_existing_behavior_by_default() {
        let indices = vec![
            make_volume('C', 2, IndexState::Ready),
            make_volume('D', 1, IndexState::Scanning),
        ];
        let policy = IpcSecurityPolicy {
            redact_status_metrics: false,
        };

        let status = build_status_response(&indices, &policy);
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
        let policy = IpcSecurityPolicy {
            redact_status_metrics: true,
        };

        let status = build_status_response(&indices, &policy);
        assert_eq!(status.total_files_indexed, 0);
        assert_eq!(status.volumes.len(), 2);
        assert_eq!(status.volumes[0].state, "redacted");
        assert_eq!(status.volumes[0].files_indexed, 0);
        assert_eq!(status.volumes[1].state, "redacted");
    }
}
