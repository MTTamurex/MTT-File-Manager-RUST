mod child_index;
mod device_notifications;
mod disk_analysis;
mod drive_health;
mod file_index;
mod fs_walker;
mod index_db;
mod indexing_progress;
mod ipc_authorization;
mod ipc_server;
mod memory_trim;
mod mft_reader;
mod name_arena;
mod path_resolver;
mod record_store;
mod security_policy;
mod service_control;
mod usn_journal;
mod volume_indexers;
mod volume_indices;
mod volume_lifecycle;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
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
///
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

    memory_trim::start_coordinator(shutdown.clone());

    // Create shared state before anything else so the IPC server can start
    // accepting connections immediately — even before volumes are discovered.
    let indices: SharedVolumeIndices = volume_indices::new_shared();
    let indexing_progress = Arc::new(indexing_progress::IndexingProgress::new());
    let (discovery_wake, discovery_events) = mpsc::channel();
    let volume_lifecycle = volume_lifecycle::VolumeLifecycle::new(
        shutdown.clone(),
        indices.clone(),
        indexing_progress.clone(),
        discovery_wake,
    );

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

    // Log baseline cache sizes without delaying IPC startup.
    std::thread::spawn(index_db::diagnose_cache_size);

    let device_notifications = match device_notifications::register_volume_notifications(
        volume_lifecycle.clone(),
    ) {
        Ok(registration) => Some(registration),
        Err(error) => {
            eprintln!(
                "[SERVICE] Volume indexing disabled because PnP removal notifications could not be registered: {}",
                redact_paths(&error)
            );
            None
        }
    };

    if device_notifications.is_some() {
        let indices = indices.clone();
        let indexing_progress = indexing_progress.clone();
        let lifecycle = volume_lifecycle.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();
        std::thread::spawn(move || {
            let mut handle_notifications =
                device_notifications::VolumeHandleNotifications::default();
            let discovered =
                usn_journal::discover_volumes(|letter| lifecycle.is_drive_suspended(letter));
            log_discovered_volumes(&discovered);
            spawn_indexers_for_discovered_volumes(
                discovered,
                &lifecycle,
                &indices,
                &indexing_progress,
                &db,
                &mut handle_notifications,
            );

            const DISCOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);
            loop {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                match discovery_events.recv_timeout(DISCOVERY_INTERVAL) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if shutdown.load(Ordering::Acquire) {
                    break;
                }

                let discovered =
                    usn_journal::discover_volumes(|letter| lifecycle.is_drive_suspended(letter));
                spawn_indexers_for_discovered_volumes(
                    discovered,
                    &lifecycle,
                    &indices,
                    &indexing_progress,
                    &db,
                    &mut handle_notifications,
                );
            }
        });
    }

    // Start IPC server immediately (blocks until shutdown)
    eprintln!(
        "[SERVICE] Starting IPC server on {}...",
        mtt_search_protocol::PIPE_NAME
    );
    ipc_server::run_ipc_server(
        indices.clone(),
        indexing_progress.clone(),
        shutdown.clone(),
        volume_lifecycle,
    );
    drop(device_notifications);
    eprintln!("[SERVICE] Shutting down...");
}

fn log_discovered_volumes(discovered: &[usn_journal::DiscoveredVolume]) {
    if discovered.is_empty() {
        eprintln!("[SERVICE] No accessible volumes found at startup.");
        return;
    }

    for volume in discovered {
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

fn spawn_indexers_for_discovered_volumes(
    discovered: Vec<usn_journal::DiscoveredVolume>,
    lifecycle: &Arc<volume_lifecycle::VolumeLifecycle>,
    indices: &SharedVolumeIndices,
    indexing_progress: &Arc<indexing_progress::IndexingProgress>,
    db: &Arc<index_db::IndexDb>,
    handle_notifications: &mut device_notifications::VolumeHandleNotifications,
) {
    lifecycle.reconcile_discovered_volumes(&discovered);
    handle_notifications.reconcile(&discovered);

    for volume in discovered {
        if volume.volume_id.is_empty() {
            continue;
        }
        if let Err(error) = handle_notifications.ensure(&volume, lifecycle) {
            eprintln!(
                "[PNP] Skipping indexer for {}: {}",
                volume.drive_letter,
                redact_paths(&error)
            );
            continue;
        }
        volume_indexers::spawn_volume_indexer(
            volume,
            lifecycle,
            indices.clone(),
            indexing_progress.clone(),
            db.clone(),
        );
    }
}
