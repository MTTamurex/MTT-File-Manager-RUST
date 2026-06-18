use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::hint::black_box;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

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
const LIVE_SIZE_REFRESH_FILE_LIMIT: u64 = 4_096;
const FRN_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

#[inline]
fn file_index_to_frn(high: u32, low: u32) -> u64 {
    (((high as u64) << 32) | low as u64) & FRN_MASK
}

fn is_absolute_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn live_directory_frn(path: &str) -> Result<u64, String> {
    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;

    if !is_absolute_drive_path(path) {
        return Err("Path is not an absolute drive path".to_string());
    }

    let path_wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(
                FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0,
            ),
            None,
        )
    }
    .map_err(|error| format!("CreateFileW failed: {}", error))?;
    let _guard = HandleGuard(handle);

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(handle, &mut info)
            .map_err(|error| format!("GetFileInformationByHandle failed: {}", error))?;
    }

    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Err("Path is not a directory".to_string());
    }

    Ok(file_index_to_frn(info.nFileIndexHigh, info.nFileIndexLow))
}

fn repair_suspicious_zero_folder_size(
    handle: &VolumeIndexHandle,
    drive_letter: char,
    path: &str,
    dir_frn: u64,
    summary: (u64, u64, u64, u64),
    has_pending_refreshes: bool,
) -> (u64, u64, u64) {
    let (total_size, file_count, folder_count, zero_size_count) = summary;

    // Determine whether a repair is warranted:
    //   (a) Small subtrees: refresh live sizes to catch files that grew after
    //       indexing but no longer have a pending USN refresh.
    //   (b) total=0 with files present: all sizes are missing from the index.
    //   (c) has_pending_refreshes=true AND >=25% of files (min 5) have zero size:
    //       post-restart race condition where USN catch-up inserted new file FRNs
    //       (size=0) before the background pending_size_refresh task ran, causing
    //       folder totals to be severely underreported (e.g. 185 KB vs 1.30 GB).
    let needs_repair = if file_count > 0 && file_count <= LIVE_SIZE_REFRESH_FILE_LIMIT {
        true
    } else if total_size == 0 {
        file_count > 0
    } else {
        has_pending_refreshes
            && zero_size_count >= 5
            && zero_size_count.saturating_mul(4) >= file_count
    };

    if !needs_repair {
        return (total_size, file_count, folder_count);
    }

    let volume = match crate::usn_journal::open_volume(drive_letter) {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!(
                "[FOLDER-SIZE] live size refresh open-volume failed path={} reason={}",
                crate::redact_paths(path),
                crate::redact_paths(&error),
            );
            return (total_size, file_count, folder_count);
        }
    };

    let record_size = match crate::mft_reader::query_mft_geometry_pub(volume) {
        Ok(record_size) => record_size,
        Err(error) => {
            crate::usn_journal::close_volume(volume);
            eprintln!(
                "[FOLDER-SIZE] live size refresh geometry failed path={} reason={}",
                crate::redact_paths(path),
                crate::redact_paths(&error),
            );
            return (total_size, file_count, folder_count);
        }
    };

    let candidates: Vec<(u64, Option<String>)> = {
        let vol = handle.read();
        let live_refresh_candidates = if file_count <= LIVE_SIZE_REFRESH_FILE_LIMIT {
            vol.collect_file_frns_in_subtree_limited(dir_frn, LIVE_SIZE_REFRESH_FILE_LIMIT as usize)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let candidates = if live_refresh_candidates.is_empty() {
            vol.collect_zero_size_file_frns_in_subtree(dir_frn, ZERO_SIZE_FOLDER_REPAIR_LIMIT)
        } else {
            live_refresh_candidates
        };

        let mut dir_cache = HashMap::new();
        candidates
            .into_iter()
            .map(|frn| {
                let path = crate::path_resolver::resolve_path_cached(frn, &vol, &mut dir_cache);
                (frn, path)
            })
            .collect()
    };

    let candidate_count = candidates.len();
    let (changed_count, refreshed_summary) = if candidates.is_empty() {
        (0usize, (total_size, file_count, folder_count))
    } else {
        let mut size_updates: Vec<(u64, u64)> = Vec::with_capacity(candidates.len());
        for (frn, resolved_path) in &candidates {
            let size =
                crate::mft_reader::read_single_file_size(volume, *frn, record_size).or_else(|| {
                    resolved_path.as_ref().and_then(|path| {
                        let path = if path.starts_with("\\\\?\\") {
                            path.clone()
                        } else {
                            format!(r"\\?\{}", path)
                        };
                        std::fs::metadata(path)
                            .ok()
                            .filter(|metadata| metadata.is_file())
                            .map(|metadata| metadata.len())
                    })
                });

            if let Some(size) = size {
                size_updates.push((*frn, size));
            }
        }

        if size_updates.is_empty() {
            (0usize, (total_size, file_count, folder_count))
        } else {
            let mut vol = handle.write();
            let mut changed = 0usize;
            let mut binary_dirty = false;
            for (frn, size) in size_updates {
                if let Some(record) = vol.records.get_mut(&frn) {
                    if !record.is_dir && record.size != size {
                        record.size = size;
                        changed += 1;
                        binary_dirty = true;
                    }
                }
            }
            if binary_dirty {
                vol.binary_dirty = true;
            }
            let (rt, rfc, rfoldc, _) = crate::mft_reader::folder_size_for_service(&vol, dir_frn);
            (changed, (rt, rfc, rfoldc))
        }
    };

    crate::usn_journal::close_volume(volume);

    if changed_count > 0 {
        eprintln!(
            "[FOLDER-SIZE] refreshed-file-sizes path={} candidates={} changed={} total_gb={:.2} files={} folders={}",
            crate::redact_paths(path),
            candidate_count,
            changed_count,
            refreshed_summary.0 as f64 / 1_073_741_824.0,
            refreshed_summary.1,
            refreshed_summary.2,
        );
        return refreshed_summary;
    }

    if candidate_count > 0 {
        eprintln!(
            "[FOLDER-SIZE] live size refresh no-change path={} candidates={} total_gb={:.2} files={} folders={}",
            crate::redact_paths(path),
            candidate_count,
            total_size as f64 / 1_073_741_824.0,
            file_count,
            folder_count,
        );
    }

    (total_size, file_count, folder_count)
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
                                    vol.names.for_each_slice(|arena_slice| {
                                        for chunk in arena_slice.chunks(4096) {
                                            black_box(&chunk[0]);
                                        }
                                    });
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

            // Always use in-memory search with a tiny reusable lowercase buffer
            // for ASCII SIMD matching.
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

            let (result, has_pending_refreshes) = {
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
                // Capture whether there are pending size refreshes while we hold the
                // read lock. This is passed to repair_suspicious_zero_folder_size to
                // decide whether to repair partial-zero subtrees (post-restart race).
                let has_pending = !vol.pending_size_refresh.is_empty();
                let r = match vol.resolve_path_to_frn(&path) {
                    Some(frn) => Ok((frn, crate::mft_reader::folder_size_for_service(&vol, frn))),
                    None => Err("Path not found in index"),
                };
                (r, has_pending)
            };

            let needs_live_fallback =
                result.as_ref().err().copied() == Some("Path not found in index");
            let result = if needs_live_fallback {
                match live_directory_frn(&path) {
                    Ok(frn) => {
                        let vol = handle.read();
                        if !matches!(vol.state, IndexState::Ready) {
                            Err("Volume not ready")
                        } else if !vol.sizes_loaded {
                            Err("Sizes not loaded")
                        } else {
                            match vol.records.get(&frn) {
                                Some(record) if record.is_dir => {
                                    eprintln!(
                                        "[FOLDER-SIZE] resolved via live FRN fallback path={} frn={}",
                                        crate::redact_paths(&path),
                                        frn,
                                    );
                                    Ok((frn, crate::mft_reader::folder_size_for_service(&vol, frn)))
                                }
                                Some(_) => Err("Path resolved to non-directory record"),
                                None => Err("Path FRN not found in index"),
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!(
                            "[FOLDER-SIZE] live FRN fallback failed path={} reason={}",
                            crate::redact_paths(&path),
                            crate::redact_paths(&error),
                        );
                        Err("Path not found in index")
                    }
                }
            } else {
                result
            };

            match result {
                Ok((dir_frn, summary)) => {
                    let (total_size, file_count, folder_count) = repair_suspicious_zero_folder_size(
                        &handle,
                        drive_letter,
                        &path,
                        dir_frn,
                        summary,
                        has_pending_refreshes,
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
    fn file_index_to_frn_strips_sequence_number() {
        assert_eq!(
            super::file_index_to_frn(0x1234_5678, 0x9ABC_DEF0),
            0x0000_5678_9ABC_DEF0
        );
    }

    #[test]
    fn live_fallback_accepts_only_absolute_drive_paths() {
        assert!(super::is_absolute_drive_path(r"C:\data"));
        assert!(super::is_absolute_drive_path("d:/data"));
        assert!(!super::is_absolute_drive_path("C:relative"));
        assert!(!super::is_absolute_drive_path(r"\\server\share"));
        assert!(!super::is_absolute_drive_path(""));
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
