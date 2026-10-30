use crate::domain::file_entry::FileEntry;
use eframe::egui;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Minimum interval between folder-cover re-scans for the same directory when
/// the parent listing signature did not change.
const COVER_SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// Probes slower than this are logged at INFO level for I/O attribution.
const SLOW_PROBE_LOG_THRESHOLD_MS: u64 = 50;
/// Upper bound for the per-directory cover-scan bookkeeping map.
const COVER_SCAN_MAP_LIMIT: usize = 256;

mod content_changes;

fn cover_scan_is_due(last_scan: Option<Instant>, now: Instant) -> bool {
    match last_scan {
        Some(last_scan) => now.saturating_duration_since(last_scan) >= COVER_SCAN_INTERVAL,
        None => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsistencyProbeMode {
    ListingDrift,
    PathLiveness,
    /// Search-index query only: never enumerate directories or resolve covers.
    FolderContents,
}

pub struct ConsistencyProbeRequest {
    pub path: PathBuf,
    pub is_onedrive: bool,
    pub ui_signature: u64,
    pub show_hidden_files: bool,
    pub mode: ConsistencyProbeMode,
    /// Time window for "recently modified" checks against the search service.
    /// Used to detect subfolder content changes that do not alter parent listing shape.
    pub modified_threshold_secs: u32,
    /// (folder_path, current_cover_file_path) pairs for visible subfolders whose
    /// folder cover state should be verified. FolderContents uses only the paths
    /// for index queries; it never reads covers.
    pub folder_cover_states: Vec<(PathBuf, Option<PathBuf>)>,
}

pub struct ConsistencyProbeResult {
    pub path: PathBuf,
    pub mode: ConsistencyProbeMode,
    pub disk_signature: u64,
    pub path_vanished: bool,
    /// Folder paths whose effective folder cover changed (None->Some, Some->None, Some->Some(new)).
    pub changed_folder_covers: Vec<PathBuf>,
    /// Folder paths whose contents changed recently according to the search service.
    /// This is used to invalidate folder-size caches without forcing a full reload.
    pub changed_folder_contents: Vec<PathBuf>,
    /// False on service failure; an acknowledgement must not advance watermarks.
    pub content_check_succeeded: bool,
}

/// Spawns a background thread that performs directory consistency probes
/// for non-NTFS drives where ReadDirectoryChangesW is unreliable, plus
/// lightweight current-folder liveness checks for focus recovery and index-only
/// recursive content-change checks for the focused pane on USN filesystems.
///
/// The worker drains stale requests (only processes the most recent),
/// enumerates directories only in ListingDrift mode. FolderContents always
/// acknowledges completion, even if the service reports no changes or fails.
pub fn spawn_consistency_probe_worker(
    ctx: egui::Context,
) -> (
    mpsc::Sender<ConsistencyProbeRequest>,
    mpsc::Receiver<ConsistencyProbeResult>,
) {
    let (req_tx, req_rx) = mpsc::channel::<ConsistencyProbeRequest>();
    let (res_tx, res_rx) = mpsc::channel::<ConsistencyProbeResult>();

    if let Err(e) = std::thread::Builder::new()
        .name("consistency-probe".into())
        .spawn(move || {
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            // Per-directory cover-scan bookkeeping (single-threaded worker).
            let mut last_cover_scans: HashMap<PathBuf, Instant> = HashMap::new();

            while let Ok(request) = req_rx.recv() {
                // Drain stale requests, keeping only the latest request for each
                // distinct (mode, path). Focus restore can enqueue liveness probes
                // for both dual-panel folders; processing only the final queued
                // request would leave one panel stale.
                let mut pending = vec![request];
                while let Ok(newer) = req_rx.try_recv() {
                    if let Some(existing) = pending.iter_mut().find(|existing| {
                        existing.mode == newer.mode && existing.path == newer.path
                    }) {
                        *existing = newer;
                    } else {
                        pending.push(newer);
                    }
                }

                for latest in pending {

                let path = latest.path;
                let is_onedrive = latest.is_onedrive;
                let ui_signature = latest.ui_signature;

                if latest.mode == ConsistencyProbeMode::PathLiveness {
                    let path_vanished = directory_is_confirmed_gone(path.as_path());
                    let _ = res_tx.send(ConsistencyProbeResult {
                        path,
                        mode: latest.mode,
                        disk_signature: 0,
                        path_vanished,
                        changed_folder_covers: Vec::new(),
                        changed_folder_contents: Vec::new(),
                        content_check_succeeded: false,
                    });
                    ctx.request_repaint();
                    continue;
                }

                if latest.mode == ConsistencyProbeMode::FolderContents {
                    let content_result = content_changes::query(
                        &latest.folder_cover_states, is_onedrive, latest.modified_threshold_secs,
                    );
                    // Always acknowledge, including no changes/service failure, so
                    // the UI can bound this mode to one outstanding request.
                    let _ = res_tx.send(ConsistencyProbeResult {
                        path,
                        mode: latest.mode,
                        disk_signature: ui_signature,
                        path_vanished: false,
                        changed_folder_covers: Vec::new(),
                        content_check_succeeded: content_result.is_ok(),
                        changed_folder_contents: content_result.unwrap_or_default(),
                    });
                    ctx.request_repaint();
                    continue;
                }

                let probe_start = Instant::now();
                let disk_entries = match crate::infrastructure::windows::hdd_directory_reader::read_directory_hdd_optimized(
                    path.as_path(),
                    is_onedrive,
                    latest.show_hidden_files,
                ) {
                    Ok(entries) => entries,
                    Err(_) => {
                        // Check if directory vanished
                        if directory_is_confirmed_gone(path.as_path()) {
                            let _ = res_tx.send(ConsistencyProbeResult {
                                path,
                                mode: latest.mode,
                                disk_signature: 0,
                                path_vanished: true,
                                changed_folder_covers: Vec::new(),
                                changed_folder_contents: Vec::new(),
                                content_check_succeeded: false,
                            });
                            ctx.request_repaint();
                        }
                        continue;
                    }
                };

                let disk_signature = compute_entries_signature(&disk_entries);
                let signature_changed = disk_signature != ui_signature;

                // Folder-cover re-resolution enumerates one directory per visible
                // subfolder. That is the heaviest part of the probe and, on an HDD
                // or virtual drive (Cryptomator), it used to be paid on every
                // interval even when the parent listing had not changed. Re-scan
                // only when the listing changed, or on a coarse periodic throttle.
                let now = Instant::now();
                let scan_covers = signature_changed
                    || cover_scan_is_due(last_cover_scans.get(&path).copied(), now);
                let scanned_covers = if scan_covers {
                    last_cover_scans.insert(path.clone(), now);
                    if last_cover_scans.len() > COVER_SCAN_MAP_LIMIT {
                        last_cover_scans.retain(|_, scanned_at| {
                            now.saturating_duration_since(*scanned_at) < COVER_SCAN_INTERVAL * 4
                        });
                    }
                    latest.folder_cover_states.len()
                } else {
                    0
                };

                // Re-resolve folder covers for currently visible subfolders so non-USN
                // filesystems can detect cover changes even when folder listings themselves
                // did not change.
                let changed_folder_covers: Vec<PathBuf> = if scan_covers {
                    latest
                        .folder_cover_states
                        .iter()
                        .filter_map(|(folder_path, current_cover)| {
                            let discovered_cover =
                                crate::infrastructure::windows::find_folder_preview_item(
                                    folder_path,
                                );
                            if discovered_cover != *current_cover {
                                Some(folder_path.clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                };

                // Also probe visible subfolders for recent content changes via
                // the NTFS search service. This catches size changes that do
                // not affect parent listing signature (same child names/mtimes).
                let content_result = content_changes::query(
                    &latest.folder_cover_states, is_onedrive, latest.modified_threshold_secs,
                );
                let content_check_succeeded = content_result.is_ok();
                let changed_folder_contents = content_result.unwrap_or_default();

                let has_cover_changes = !changed_folder_covers.is_empty();
                let has_folder_content_changes = !changed_folder_contents.is_empty();

                let probe_ms = probe_start.elapsed().as_millis() as u64;
                crate::infrastructure::io_trace::record_consistency_probe(
                    probe_ms,
                    scanned_covers as u64,
                );
                if probe_ms >= SLOW_PROBE_LOG_THRESHOLD_MS {
                    log::info!(
                        "[PROBE-WORKER] slow probe {}ms entries={} cover_scans={} path={:?}",
                        probe_ms,
                        disk_entries.len(),
                        scanned_covers,
                        path.file_name().unwrap_or_default()
                    );
                }

                log::debug!(
                    "[PROBE-WORKER] path={:?} entries={} sig_match={} changed_folder_covers={} changed_folder_contents={}",
                    path.file_name().unwrap_or_default(),
                    disk_entries.len(),
                    !signature_changed,
                    changed_folder_covers.len(),
                    changed_folder_contents.len()
                );

                if signature_changed || has_cover_changes || has_folder_content_changes {
                    let _ = res_tx.send(ConsistencyProbeResult {
                        path,
                        mode: latest.mode,
                        disk_signature,
                        path_vanished: false,
                        changed_folder_covers,
                        changed_folder_contents,
                        content_check_succeeded,
                    });
                    ctx.request_repaint();
                }
                }
            }
        })
    {
        log::error!("[CONSISTENCY-PROBE] Failed to spawn worker thread: {e}. Consistency probing disabled.");
    }

    (req_tx, res_rx)
}

fn directory_is_confirmed_gone(path: &std::path::Path) -> bool {
    if crate::domain::special_paths::is_virtual_path(&path.to_string_lossy()) {
        return false;
    }
    if let Some((archive_path, _)) = crate::domain::file_entry::split_archive_path(path) {
        return !archive_path.exists();
    }

    match std::fs::metadata(path) {
        Ok(metadata) => {
            if metadata.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(crate::domain::file_entry::is_archive_extension)
            {
                return false;
            }
            !metadata.is_dir()
        }
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Computes an order-independent signature over directory entries.
/// Uses XOR + wrapping-add for collision resistance without requiring sort.
fn compute_entries_signature(entries: &[FileEntry]) -> u64 {
    let mut xor_acc = 0u64;
    let mut sum_acc = 0u64;
    let mut bytes_acc = 0u64;

    for entry in entries {
        let mut hasher = DefaultHasher::new();
        entry.name.hash(&mut hasher);
        entry.is_dir.hash(&mut hasher);
        entry.size.hash(&mut hasher);
        entry.modified.hash(&mut hasher);
        let entry_hash = hasher.finish();

        xor_acc ^= entry_hash;
        sum_acc = sum_acc.wrapping_add(entry_hash);
        bytes_acc = bytes_acc.wrapping_add(entry.size);
    }

    let mut final_hasher = DefaultHasher::new();
    entries.len().hash(&mut final_hasher);
    xor_acc.hash(&mut final_hasher);
    sum_acc.hash(&mut final_hasher);
    bytes_acc.hash(&mut final_hasher);
    final_hasher.finish()
}

#[cfg(test)]
mod tests;
