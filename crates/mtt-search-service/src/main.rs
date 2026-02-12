mod file_index;
mod index_db;
mod ipc_server;
mod path_resolver;
mod service_control;
mod usn_journal;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// Redact filesystem paths from error messages to prevent information leakage.
/// Replaces tokens that look like paths (containing `\` or starting with a drive letter)
/// with `<path>`.
fn redact_paths(msg: &str) -> String {
    msg.split_whitespace()
        .map(|word| {
            let trimmed = word.trim_matches(|c: char| c == '\'' || c == '"' || c == ':');
            if trimmed.contains('\\')
                || trimmed.contains('/')
                    && trimmed.len() > 1
                    && !trimmed.starts_with("http")
            {
                "<path>"
            } else {
                word
            }
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(|s| s.as_str()) {
        Some("install") => {
            service_control::install_service();
        }
        Some("uninstall") => {
            service_control::uninstall_service();
        }
        Some("run-console") => {
            eprintln!("[SERVICE] Running in console mode (press Ctrl+C to stop)...");
            let shutdown = Arc::new(AtomicBool::new(false));
            let shutdown_clone = shutdown.clone();

            let _ = ctrlc_handler(shutdown_clone);

            run_indexer(shutdown);
        }
        _ => {
            // Normal service dispatch (called by Windows SCM)
            if let Err(e) = service_control::run_as_service() {
                eprintln!("[SERVICE] Failed to start service dispatcher: {}", e);
                eprintln!("[SERVICE] If running from command line, use one of:");
                eprintln!("  mtt-search-service.exe install       - Install as Windows service");
                eprintln!("  mtt-search-service.exe uninstall     - Remove Windows service");
                eprintln!("  mtt-search-service.exe run-console   - Run in console mode");
            }
        }
    }
}

fn ctrlc_handler(_shutdown: Arc<AtomicBool>) -> Result<(), String> {
    // Simple Ctrl+C handler for console mode
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            // In production, hook SetConsoleCtrlHandler. For now, the process
            // will just terminate on Ctrl+C which is acceptable for debugging.
        }
    });
    Ok(())
}

/// Main indexer loop. Shared between console mode and service mode.
pub fn run_indexer(shutdown: Arc<AtomicBool>) {
    eprintln!("[SERVICE] Starting indexer...");

    // Discover NTFS volumes
    let volumes = usn_journal::discover_ntfs_volumes();
    if volumes.is_empty() {
        eprintln!("[SERVICE] No NTFS volumes found.");
        return;
    }
    for (letter, label) in &volumes {
        eprintln!("[SERVICE] Found NTFS volume: {}:\\ ({})", letter, label);
    }

    // Create shared index
    let indices = Arc::new(RwLock::new(Vec::new()));

    // Load persisted index or do fresh scan for each volume
    let db_path = index_db::get_db_path();
    eprintln!("[SERVICE] Index database ready");
    let db = match index_db::IndexDb::open(&db_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!("[SERVICE] Failed to open index database: {}", redact_paths(&e.to_string()));
            return;
        }
    };

    // Index each volume
    for (drive_letter, _label) in &volumes {
        let letter = *drive_letter;
        let indices = indices.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();

        std::thread::spawn(move || {
            index_volume(letter, indices, db, shutdown);
        });
    }

    // Start IPC server (blocks until shutdown)
    eprintln!(
        "[SERVICE] Starting IPC server on {}...",
        mtt_search_protocol::PIPE_NAME
    );
    ipc_server::run_ipc_server(indices.clone(), shutdown.clone());

    eprintln!("[SERVICE] Shutting down...");
}

fn index_volume(
    drive_letter: char,
    indices: Arc<RwLock<Vec<file_index::VolumeIndex>>>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
) {
    eprintln!("[USN] Starting indexing for volume {}:\\", drive_letter);

    // Try to load cached state from database
    let cached_state = db.load_volume_state(drive_letter);
    let cached_records = if cached_state.is_some() {
        db.load_file_records(drive_letter)
    } else {
        None
    };

    // Open volume handle
    let volume_handle = match usn_journal::open_volume(drive_letter) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[USN] Failed to open volume {}:\\: {}", drive_letter, e);
            return;
        }
    };

    // Query USN Journal
    let journal_info = match usn_journal::query_usn_journal(volume_handle) {
        Ok(info) => info,
        Err(e) => {
            eprintln!(
                "[USN] Failed to query USN journal for {}:\\: {}",
                drive_letter, e
            );
            usn_journal::close_volume(volume_handle);
            return;
        }
    };

    eprintln!(
        "[USN] {}:\\ Journal ID: {}, Next USN: {}",
        drive_letter, journal_info.journal_id, journal_info.next_usn
    );

    let mut index = file_index::VolumeIndex::new(drive_letter);
    let need_full_scan;

    // Check if we can use cached data
    if let (Some(state), Some(records)) = (cached_state, cached_records) {
        if state.journal_id == journal_info.journal_id {
            eprintln!(
                "[USN] {}:\\ Loading {} cached records, catching up from USN {}...",
                drive_letter,
                records.len(),
                state.last_usn
            );
            index.records = records;
            index.journal_id = state.journal_id;
            index.last_usn = state.last_usn;

            // Catch up from last USN
            match usn_journal::read_usn_changes(
                volume_handle,
                &journal_info,
                index.last_usn,
                &mut index,
            ) {
                Ok(new_usn) => {
                    index.last_usn = new_usn;
                    eprintln!(
                        "[USN] {}:\\ Caught up to USN {}, {} total records",
                        drive_letter,
                        new_usn,
                        index.records.len()
                    );
                    need_full_scan = false;
                }
                Err(e) => {
                    eprintln!(
                        "[USN] {}:\\ Catch-up failed ({}), doing full scan",
                        drive_letter, e
                    );
                    index.records.clear();
                    need_full_scan = true;
                }
            }
        } else {
            eprintln!(
                "[USN] {}:\\ Journal ID changed ({} -> {}), full re-scan needed",
                drive_letter, state.journal_id, journal_info.journal_id
            );
            need_full_scan = true;
        }
    } else {
        need_full_scan = true;
    }

    // Full MFT enumeration if needed
    if need_full_scan {
        index.state = file_index::IndexState::Scanning;
        eprintln!("[USN] {}:\\ Starting full MFT enumeration...", drive_letter);
        let start = std::time::Instant::now();

        match usn_journal::enumerate_all_files(volume_handle, &journal_info, &mut index) {
            Ok(()) => {
                let elapsed = start.elapsed();
                index.journal_id = journal_info.journal_id;
                index.last_usn = journal_info.next_usn;
                eprintln!(
                    "[USN] {}:\\ Enumerated {} files in {:.2}s",
                    drive_letter,
                    index.records.len(),
                    elapsed.as_secs_f64()
                );
            }
            Err(e) => {
                eprintln!("[USN] {}:\\ Enumeration failed: {}", drive_letter, e);
                index.state = file_index::IndexState::Error(e);
                usn_journal::close_volume(volume_handle);
                return;
            }
        }

        // Persist to database
        if let Err(e) = db.save_volume(&index) {
            eprintln!("[USN] {}:\\ Failed to save index: {}", drive_letter, redact_paths(&e.to_string()));
        }
    }

    index.state = file_index::IndexState::Ready;
    let mut current_usn = index.last_usn;

    // Add to shared indices
    {
        let mut indices_lock = indices.write().unwrap_or_else(|e| e.into_inner());
        indices_lock.push(index);
    }

    eprintln!(
        "[USN] {}:\\ Index ready, starting incremental updates",
        drive_letter
    );

    // Incremental update loop
    let mut last_persist = std::time::Instant::now();
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        std::thread::sleep(std::time::Duration::from_secs(2));

        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        // 1. Read raw USN buffer (I/O) — NO lock held, so searches are never blocked.
        match usn_journal::read_usn_buffer(volume_handle, &journal_info, current_usn) {
            Ok(Some((buffer, bytes_returned, new_usn))) => {
                // 2. Brief WRITE lock only for applying parsed changes to the HashMap.
                //    Use try_write to avoid blocking search reads: if a search or warm is
                //    holding a read lock, skip this cycle (retry in 2s). A pending write
                //    on Windows SRWLock blocks ALL new reads, which cascades into timeouts.
                if let Ok(mut indices_lock) = indices.try_write() {
                    if let Some(vol_index) = indices_lock
                        .iter_mut()
                        .find(|v| v.drive_letter == drive_letter)
                    {
                        let mut dummy_count = 0;
                        usn_journal::parse_usn_records(
                            &buffer[8..bytes_returned as usize],
                            vol_index,
                            &mut dummy_count,
                            true,
                        );
                        vol_index.last_usn = new_usn;
                    }
                    current_usn = new_usn;
                }
                // else: lock busy (search running), skip — will retry next cycle
            }
            Ok(None) => {} // No new records
            Err(e) => {
                eprintln!("[USN] {}:\\ Incremental read error: {}", drive_letter, e);
            }
        }

        // 3. Persist every 5 minutes — under READ lock (save_volume takes &VolumeIndex).
        if last_persist.elapsed() > std::time::Duration::from_secs(300) {
            let indices_lock = indices.read().unwrap_or_else(|e| e.into_inner());
            if let Some(vol_index) = indices_lock
                .iter()
                .find(|v| v.drive_letter == drive_letter)
            {
                if let Err(e) = db.save_volume(vol_index) {
                    eprintln!("[USN] {}:\\ Persist error: {}", drive_letter, redact_paths(&e.to_string()));
                }
            }
            last_persist = std::time::Instant::now();
        }
    }

    usn_journal::close_volume(volume_handle);
    eprintln!("[USN] {}:\\ Indexer stopped", drive_letter);
}
