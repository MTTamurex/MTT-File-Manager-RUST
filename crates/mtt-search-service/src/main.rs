mod file_index;
mod fs_walker;
mod index_db;
mod indexing_progress;
mod ipc_authorization;
mod ipc_server;
mod mft_reader;
mod name_arena;
mod path_resolver;
mod security_policy;
mod service_control;
mod usn_journal;
mod volume_indexers;
mod volume_indices;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use parking_lot::Mutex;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::volume_indices::SharedVolumeIndices;

/// Redact filesystem paths from error messages to prevent information leakage.
/// SEC: Detects path-like substrings using multiple signals so we redact even
/// when paths are quoted, embedded in parentheses, or split by colons:
///   * any token containing `\` (Windows separators)
///   * any token containing `/` (POSIX-style separators) that is not a URL
///   * any token starting with a Windows drive letter (`X:`)
///   * any token starting with the verbatim/UNC prefix (`\\?\` or `\\.\`)
///   * any token containing `\\` (UNC root)
/// The redaction is conservative: when in doubt, replace with `<path>`.
pub(crate) fn redact_paths(msg: &str) -> String {
    fn looks_like_path(token: &str) -> bool {
        if token.len() < 2 {
            return false;
        }
        // Strip surrounding quotes/brackets/punctuation that commonly wrap paths
        // in human-readable error strings.
        let stripped = token.trim_matches(|c: char| {
            matches!(
                c,
                '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '.'
            )
        });
        if stripped.starts_with("http://") || stripped.starts_with("https://") {
            return false;
        }
        if stripped.starts_with(r"\\?\") || stripped.starts_with(r"\\.\") {
            return true;
        }
        if stripped.contains('\\') {
            return true;
        }
        if stripped.contains('/') && stripped.len() > 2 {
            return true;
        }
        // Drive letter form: `C:` or `C:\foo` or `C:/foo`.
        let bytes = stripped.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            return true;
        }
        false
    }

    msg.split_whitespace()
        .map(|word| {
            if looks_like_path(word) {
                "<path>"
            } else {
                word
            }
        })
        .collect::<Vec<&str>>()
        .join(" ")
}

fn main() {
    // SEC: Remove the current working directory from the default DLL search order.
    // Prevents DLL planting attacks when running as LocalSystem.
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::LibraryLoader::SetDefaultDllDirectories;
        use windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;
        if let Err(error) = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) {
            eprintln!(
                "[SERVICE] DLL search hardening failed: {} (service continues with reduced hardening)",
                error
            );
        }
    }

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
                eprintln!(
                    "  mtt-search-service.exe install                 - Install as LocalSystem"
                );
                eprintln!(
                    "  mtt-search-service.exe uninstall               - Remove Windows service"
                );
                eprintln!("  mtt-search-service.exe run-console             - Run in console mode");
            }
        }
    }
}

/// Global flag set by the console ctrl handler callback.
static CONSOLE_SHUTDOWN: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn console_ctrl_callback(_ctrl_type: u32) -> windows::core::BOOL {
    CONSOLE_SHUTDOWN.store(true, Ordering::SeqCst);
    true.into()
}

fn ctrlc_handler(shutdown: Arc<AtomicBool>) -> Result<(), String> {
    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(Some(console_ctrl_callback), true)
            .map_err(|e| format!("SetConsoleCtrlHandler failed: {}", e))?;
    }

    // Propagate the static flag to the shared shutdown Arc.
    std::thread::spawn(move || loop {
        if CONSOLE_SHUTDOWN.load(Ordering::Relaxed) {
            eprintln!("[SERVICE] Ctrl+C received, shutting down...");
            shutdown.store(true, Ordering::SeqCst);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    Ok(())
}

/// Main indexer loop. Shared between console mode and service mode.
pub fn run_indexer(shutdown: Arc<AtomicBool>) {
    eprintln!("[SERVICE] mtt-search-service v2 (compact-arena index)");
    eprintln!(
        "[SERVICE] FileRecord size: {} bytes",
        std::mem::size_of::<file_index::FileRecord>()
    );
    eprintln!("[SERVICE] Starting indexer...");

    // Create shared state before anything else so the IPC server can start
    // accepting connections immediately — even before volumes are discovered.
    let indices: SharedVolumeIndices = volume_indices::new_shared();
    let indexing_progress = Arc::new(indexing_progress::IndexingProgress::new());

    // Open persistence (needed by both IPC and indexers)
    let db_path = match index_db::get_db_path() {
        Ok(path) => path,
        Err(e) => {
            eprintln!(
                "[SERVICE] Secure persistence path initialization failed: {}",
                redact_paths(&e)
            );
            return;
        }
    };
    eprintln!("[SERVICE] Index database ready: {}", db_path.display());
    let db = match index_db::IndexDb::open(&db_path) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            eprintln!(
                "[SERVICE] Failed to open index database: {}",
                redact_paths(&e.to_string())
            );
            return;
        }
    };

    // Spawn volume discovery + indexing in a background thread.
    // This allows the IPC server to start listening immediately instead of
    // blocking on discover_volumes() which can take seconds if network or
    // optical drives are mounted.
    {
        let indices = indices.clone();
        let indexing_progress = indexing_progress.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();
        std::thread::spawn(move || {
            let tracked_volumes = Arc::new(Mutex::new(HashSet::<char>::new()));

            let discovered = usn_journal::discover_volumes();
            if discovered.is_empty() {
                eprintln!("[SERVICE] No accessible volumes found at startup.");
            } else {
                for volume in &discovered {
                    if volume.usn_supported {
                        eprintln!(
                            "[SERVICE] Found USN-capable volume: {}:\\ ({}, {})",
                            volume.drive_letter, volume.label, volume.file_system
                        );
                    } else {
                        eprintln!(
                            "[SERVICE] Found fallback volume: {}:\\ ({}, {})",
                            volume.drive_letter, volume.label, volume.file_system
                        );
                    }
                }
            }

            spawn_indexers_for_discovered_volumes(
                discovered,
                &tracked_volumes,
                &indices,
                &indexing_progress,
                &db,
                &shutdown,
            );

            // Keep discovering newly mounted drives (e.g., Cryptomator mounts).
            const DISCOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
            loop {
                if volume_indexers::wait_for_shutdown_or_timeout(&shutdown, DISCOVERY_INTERVAL) {
                    break;
                }

                let discovered = usn_journal::discover_volumes();
                spawn_indexers_for_discovered_volumes(
                    discovered,
                    &tracked_volumes,
                    &indices,
                    &indexing_progress,
                    &db,
                    &shutdown,
                );
            }
        });
    }

    // Start IPC server immediately (blocks until shutdown)
    eprintln!(
        "[SERVICE] Starting IPC server on {}...",
        mtt_search_protocol::PIPE_NAME
    );
    ipc_server::run_ipc_server(indices.clone(), indexing_progress.clone(), shutdown.clone());
    eprintln!("[SERVICE] Shutting down...");
}

fn spawn_indexers_for_discovered_volumes(
    discovered: Vec<usn_journal::DiscoveredVolume>,
    tracked_volumes: &Arc<Mutex<HashSet<char>>>,
    indices: &SharedVolumeIndices,
    indexing_progress: &Arc<indexing_progress::IndexingProgress>,
    db: &Arc<index_db::IndexDb>,
    shutdown: &Arc<AtomicBool>,
) {
    for volume in discovered {
        let drive_letter = volume.drive_letter;
        let should_spawn = {
            let mut tracked = tracked_volumes.lock();
            tracked.insert(drive_letter)
        };

        if !should_spawn {
            continue;
        }

        spawn_volume_indexer(
            volume,
            tracked_volumes.clone(),
            indices.clone(),
            indexing_progress.clone(),
            db.clone(),
            shutdown.clone(),
        );
    }
}

fn spawn_volume_indexer(
    volume: usn_journal::DiscoveredVolume,
    tracked_volumes: Arc<Mutex<HashSet<char>>>,
    indices: SharedVolumeIndices,
    indexing_progress: Arc<indexing_progress::IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
) {
    let drive_letter = volume.drive_letter;
    let label = if volume.label.is_empty() {
        "(no label)"
    } else {
        volume.label.as_str()
    };

    if volume.usn_supported {
        eprintln!(
            "[SERVICE] Starting USN indexer for {}:\\ ({}, {})",
            drive_letter, label, volume.file_system
        );
    } else {
        eprintln!(
            "[SERVICE] Starting fallback scanner for {}:\\ ({}, {})",
            drive_letter, label, volume.file_system
        );
    }

    std::thread::spawn(move || {
        if volume.usn_supported {
            volume_indexers::index_volume(drive_letter, indices, indexing_progress, db, shutdown);
        } else {
            volume_indexers::index_non_ntfs_volume(
                drive_letter,
                volume.file_system,
                indices,
                indexing_progress,
                db,
                shutdown,
            );
        }

        let mut tracked = tracked_volumes.lock();
        tracked.remove(&drive_letter);
    });
}
