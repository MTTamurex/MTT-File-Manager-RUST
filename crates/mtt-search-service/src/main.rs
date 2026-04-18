mod file_index;
mod fs_walker;
mod indexing_progress;
mod index_db;
mod ipc_authorization;
mod ipc_server;
mod mft_reader;
mod name_arena;
mod path_resolver;
mod security_policy;
mod service_control;
mod usn_journal;
mod volume_indexers;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::{Mutex, RwLock};

/// Tracks whether the FTS5 index is in sync with `file_records`.
///
/// Volume saves strip FTS ops for speed, then rebuild FTS in a background
/// thread.  Between save and rebuild completion, the IPC handler falls back
/// to the in-memory linear scan.
///
/// A generation counter prevents a stale rebuild (started before a second
/// volume save) from prematurely marking the index as ready.
pub(crate) struct FtsState {
    ready: AtomicBool,
    generation: AtomicU64,
}

impl FtsState {
    pub fn new() -> Self {
        Self {
            ready: AtomicBool::new(true),
            generation: AtomicU64::new(0),
        }
    }

    /// Mark FTS as stale (a new volume save is about to replace records).
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.ready.store(false, Ordering::Release);
    }

    /// Snapshot the current generation for a rebuild thread.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Mark FTS as ready, but only if no other save invalidated it since
    /// `expected_gen`.
    pub fn try_mark_ready(&self, expected_gen: u64) {
        if self.generation.load(Ordering::SeqCst) == expected_gen {
            self.ready.store(true, Ordering::Release);
        }
    }
}

/// Redact filesystem paths from error messages to prevent information leakage.
/// Replaces tokens that look like paths (containing `\` or starting with a drive letter)
/// with `<path>`.
pub(crate) fn redact_paths(msg: &str) -> String {
    msg.split_whitespace()
        .map(|word| {
            let trimmed = word.trim_matches(|c: char| c == '\'' || c == '"' || c == ':');
            if trimmed.contains('\\')
                || trimmed.contains('/') && trimmed.len() > 1 && !trimmed.starts_with("http")
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
                eprintln!("  mtt-search-service.exe install                 - Install as LocalSystem");
                eprintln!("  mtt-search-service.exe uninstall               - Remove Windows service");
                eprintln!("  mtt-search-service.exe run-console             - Run in console mode");
            }
        }
    }
}

/// Global flag set by the console ctrl handler callback.
static CONSOLE_SHUTDOWN: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn console_ctrl_callback(
    _ctrl_type: u32,
) -> windows::core::BOOL {
    CONSOLE_SHUTDOWN.store(true, Ordering::SeqCst);
    true.into()
}

fn ctrlc_handler(shutdown: Arc<AtomicBool>) -> Result<(), String> {
    unsafe {
        windows::Win32::System::Console::SetConsoleCtrlHandler(
            Some(console_ctrl_callback),
            true,
        )
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
    let indices = Arc::new(RwLock::new(Vec::new()));
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

    // FTS5 readiness state — shared between indexers and IPC server.
    let fts_state = Arc::new(FtsState::new());

    // Spawn volume discovery + indexing in a background thread.
    // This allows the IPC server to start listening immediately instead of
    // blocking on discover_volumes() which can take seconds if network or
    // optical drives are mounted.
    {
        let indices = indices.clone();
        let indexing_progress = indexing_progress.clone();
        let db = db.clone();
        let shutdown = shutdown.clone();
        let fts_state = fts_state.clone();

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
                &fts_state,
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
                    &fts_state,
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
        fts_state,
    );

    db.mark_clean_shutdown();
    eprintln!("[SERVICE] Shutting down...");
}

fn spawn_indexers_for_discovered_volumes(
    discovered: Vec<usn_journal::DiscoveredVolume>,
    tracked_volumes: &Arc<Mutex<HashSet<char>>>,
    indices: &Arc<RwLock<Vec<file_index::VolumeIndex>>>,
    indexing_progress: &Arc<indexing_progress::IndexingProgress>,
    db: &Arc<index_db::IndexDb>,
    shutdown: &Arc<AtomicBool>,
    fts_state: &Arc<FtsState>,
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
            fts_state.clone(),
        );
    }
}

fn spawn_volume_indexer(
    volume: usn_journal::DiscoveredVolume,
    tracked_volumes: Arc<Mutex<HashSet<char>>>,
    indices: Arc<RwLock<Vec<file_index::VolumeIndex>>>,
    indexing_progress: Arc<indexing_progress::IndexingProgress>,
    db: Arc<index_db::IndexDb>,
    shutdown: Arc<AtomicBool>,
    fts_state: Arc<FtsState>,
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
            volume_indexers::index_volume(drive_letter, indices, indexing_progress, db, shutdown, fts_state);
        } else {
            volume_indexers::index_non_ntfs_volume(
                drive_letter,
                volume.file_system,
                indices,
                indexing_progress,
                db,
                shutdown,
                fts_state,
            );
        }

        let mut tracked = tracked_volumes.lock();
        tracked.remove(&drive_letter);
    });
}

