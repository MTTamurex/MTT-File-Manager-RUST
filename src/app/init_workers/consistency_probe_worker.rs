use crate::domain::file_entry::FileEntry;
use eframe::egui;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsistencyProbeMode {
    ListingDrift,
    PathLiveness,
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
    /// folder cover state should be verified.
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
}

/// Spawns a background thread that performs directory consistency probes
/// for non-NTFS drives where ReadDirectoryChangesW is unreliable, plus
/// lightweight current-folder liveness checks for focus recovery.
///
/// The worker drains stale requests (only processes the most recent),
/// reads the directory from disk, and sends back a result only when
/// the disk signature differs from the UI signature.
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
                    });
                    ctx.request_repaint();
                    continue;
                }

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
                            });
                            ctx.request_repaint();
                        }
                        continue;
                    }
                };

                let disk_signature = compute_entries_signature(&disk_entries);

                // Re-resolve folder covers for currently visible subfolders so non-USN
                // filesystems can detect cover changes even when folder listings themselves
                // did not change.
                let changed_folder_covers: Vec<PathBuf> = latest
                    .folder_cover_states
                    .iter()
                    .filter_map(|(folder_path, current_cover)| {
                        let discovered_cover =
                            crate::infrastructure::windows::find_folder_preview_item(folder_path);
                        if discovered_cover != *current_cover {
                            Some(folder_path.clone())
                        } else {
                            None
                        }
                    })
                    .collect();

                // Also probe visible subfolders for recent content changes via
                // the NTFS search service. This catches size changes that do
                // not affect parent listing signature (same child names/mtimes).
                let changed_folder_contents: Vec<PathBuf> = if latest.is_onedrive
                    || latest.folder_cover_states.is_empty()
                {
                    Vec::new()
                } else {
                    let candidate_paths: Vec<String> = latest
                        .folder_cover_states
                        .iter()
                        .map(|(folder_path, _)| folder_path.to_string_lossy().to_string())
                        .collect();

                    if candidate_paths.is_empty() {
                        Vec::new()
                    } else {
                        let threshold = latest.modified_threshold_secs.max(5);
                        match crate::infrastructure::global_search::check_paths_modified(
                            &candidate_paths,
                            threshold,
                        ) {
                            Ok(modified_paths) => {
                                let modified_set: std::collections::HashSet<String> =
                                    modified_paths.into_iter().collect();
                                latest
                                    .folder_cover_states
                                    .iter()
                                    .filter_map(|(folder_path, _)| {
                                        let key = folder_path.to_string_lossy().to_string();
                                        if modified_set.contains(&key) {
                                            Some(folder_path.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect()
                            }
                            Err(_) => Vec::new(),
                        }
                    }
                };

                let signature_changed = disk_signature != ui_signature;
                let has_cover_changes = !changed_folder_covers.is_empty();
                let has_folder_content_changes = !changed_folder_contents.is_empty();

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
    match std::fs::metadata(path) {
        Ok(metadata) => !metadata.is_dir(),
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
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mtt_consistency_probe_{name}_{nanos}"))
    }

    #[test]
    fn liveness_check_accepts_existing_directory() {
        let path = unique_temp_path("dir");
        std::fs::create_dir(&path).unwrap();

        assert!(!directory_is_confirmed_gone(&path));

        let _ = std::fs::remove_dir(&path);
    }

    #[test]
    fn liveness_check_detects_missing_directory() {
        let path = unique_temp_path("missing");

        assert!(directory_is_confirmed_gone(&path));
    }

    #[test]
    fn liveness_check_rejects_file_replacing_directory() {
        let path = unique_temp_path("file");
        std::fs::write(&path, b"not a directory").unwrap();

        assert!(directory_is_confirmed_gone(&path));

        let _ = std::fs::remove_file(&path);
    }
}
