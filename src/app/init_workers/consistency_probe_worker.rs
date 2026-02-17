use crate::domain::file_entry::FileEntry;
use eframe::egui;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::mpsc;

pub struct ConsistencyProbeRequest {
    pub path: PathBuf,
    pub is_onedrive: bool,
    pub ui_signature: u64,
    /// (folder_path, cover_file_path) pairs for subfolders whose covers should be verified.
    pub cover_paths: Vec<(PathBuf, PathBuf)>,
}

pub struct ConsistencyProbeResult {
    pub path: PathBuf,
    pub disk_signature: u64,
    pub path_vanished: bool,
    /// Folder paths whose cover file no longer exists on disk.
    pub stale_covers: Vec<PathBuf>,
}

/// Spawns a background thread that performs directory consistency probes
/// for non-NTFS drives where ReadDirectoryChangesW is unreliable.
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

    std::thread::Builder::new()
        .name("consistency-probe".into())
        .spawn(move || {
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            while let Ok(request) = req_rx.recv() {
                // Drain stale requests — only process the most recent.
                let mut latest = request;
                while let Ok(newer) = req_rx.try_recv() {
                    latest = newer;
                }

                let path = latest.path;
                let is_onedrive = latest.is_onedrive;
                let ui_signature = latest.ui_signature;

                let disk_entries = match crate::infrastructure::windows::hdd_directory_reader::read_directory_hdd_optimized(
                    path.as_path(),
                    is_onedrive,
                ) {
                    Ok(entries) => entries,
                    Err(_) => {
                        // Check if directory vanished
                        if !path.is_dir() {
                            let _ = res_tx.send(ConsistencyProbeResult {
                                path,
                                disk_signature: 0,
                                path_vanished: true,
                                stale_covers: Vec::new(),
                            });
                            ctx.request_repaint();
                        }
                        continue;
                    }
                };

                let disk_signature = compute_entries_signature(&disk_entries);

                // Check which subfolder covers have been deleted from disk.
                let stale_covers: Vec<PathBuf> = latest
                    .cover_paths
                    .iter()
                    .filter(|(_, cover_file)| {
                        !crate::infrastructure::onedrive::fast_path_exists(cover_file)
                    })
                    .map(|(folder_path, _)| folder_path.clone())
                    .collect();

                let signature_changed = disk_signature != ui_signature;
                let has_stale_covers = !stale_covers.is_empty();

                log::debug!(
                    "[PROBE-WORKER] path={:?} entries={} sig_match={} stale_covers={}",
                    path.file_name().unwrap_or_default(),
                    disk_entries.len(),
                    !signature_changed,
                    stale_covers.len()
                );

                if signature_changed || has_stale_covers {
                    let _ = res_tx.send(ConsistencyProbeResult {
                        path,
                        disk_signature,
                        path_vanished: false,
                        stale_covers,
                    });
                    ctx.request_repaint();
                }
            }
        })
        .expect("failed to spawn consistency-probe worker");

    (req_tx, res_rx)
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
