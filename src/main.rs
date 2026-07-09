#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

// NOTE: We deliberately do NOT export the NvOptimusEnablement /
// AmdPowerXpressRequestHighPerformance symbols here.  This binary is reused as
// the image / pdf / text viewer subprocess and forcing the discrete GPU at
// process start-up wakes the dGPU even when those viewers ask wgpu for
// `PowerPreference::LowPower`, costing significant baseline RAM/VRAM per
// viewer window.  The main file-manager process still asks wgpu for
// `HighPerformance` in its NativeOptions, which on hybrid laptops is normally
// honoured by the standard DXGI adapter enumeration.

use eframe::egui;
use mtt_file_manager::app::ImageViewerApp;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

mod gpu_backend;

const APP_ID: &str = "mtt-file-manager";

fn cleanup_eframe_storage(app_id: &str) {
    let Some(mut storage_dir) = dirs::data_dir() else {
        return;
    };

    storage_dir.push(app_id);
    storage_dir.push("data");
    storage_dir.push("app.ron");

    if let Err(err) = std::fs::remove_file(&storage_dir) {
        if err.kind() != std::io::ErrorKind::NotFound {
            log::debug!(
                "[STARTUP] Failed to remove stale eframe storage at '{}': {}",
                storage_dir.display(),
                err
            );
        }
    }
}

/// Load application icon from embedded pre-sized 256x256 PNG bytes.
/// PERF: Uses a pre-sized asset to avoid the expensive CatmullRom resize at startup.
fn load_app_icon() -> Option<egui::IconData> {
    match image::load_from_memory(mtt_file_manager::embedded_assets::APP_ICON_256_PNG) {
        Ok(img) => {
            let rgba_image = img.to_rgba8();
            let (width, height) = (rgba_image.width(), rgba_image.height());
            let pixels = rgba_image.into_raw();

            Some(egui::IconData {
                rgba: pixels,
                width,
                height,
            })
        }
        Err(e) => {
            log::warn!("Failed to load embedded app icon: {}", e);
            None
        }
    }
}

/// Check whether a real stderr console handle is attached.
/// GUI-subsystem binaries launched from a shortcut (Explorer) have no console,
/// so `GetStdHandle(STD_ERROR_HANDLE)` returns NULL or INVALID_HANDLE_VALUE.
/// When launched from a terminal (`cargo run`, PowerShell), the handle is valid.
#[cfg(target_os = "windows")]
fn has_stderr_console() -> bool {
    use std::os::windows::io::AsRawHandle;
    let h = std::io::stderr().as_raw_handle() as usize;
    // NULL = 0, INVALID_HANDLE_VALUE = usize::MAX (i.e. -1 as usize)
    h != 0 && h != usize::MAX
}

/// Read startup-critical preferences with a single SQLite open before full app init.
fn read_early_preferences(keys: &[&str]) -> std::collections::HashMap<String, String> {
    let mut values = std::collections::HashMap::new();
    let Some(data_local_dir) = dirs::data_local_dir() else {
        return values;
    };
    let db_path = data_local_dir
        .join("MTT-File-Manager")
        .join("state")
        .join("app_state.db");
    if !db_path.exists() {
        return values;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok();
    let Some(conn) = conn else {
        return values;
    };

    let Ok(mut stmt) = conn.prepare("SELECT value FROM user_preferences WHERE key = ?") else {
        return values;
    };
    for key in keys {
        if let Ok(value) = stmt.query_row(rusqlite::params![key], |row| row.get::<_, String>(0)) {
            values.insert((*key).to_string(), value);
        }
    }
    values
}

#[cfg(target_os = "windows")]
fn start_temporary_startup_priority_boost() {
    use std::time::Duration;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetPriorityClass, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
        PROCESS_CREATION_FLAGS,
    };

    const STARTUP_BOOST_DURATION: Duration = Duration::from_secs(4);

    let process = unsafe { GetCurrentProcess() };
    let original_priority = unsafe { GetPriorityClass(process) };
    if original_priority == 0 {
        log::warn!("[STARTUP] Failed to read process priority class; startup boost skipped");
        return;
    }

    if let Err(error) = unsafe { SetPriorityClass(process, ABOVE_NORMAL_PRIORITY_CLASS) } {
        log::warn!("[STARTUP] Failed to enable startup priority boost: {error}");
        return;
    }

    log::info!(
        "[STARTUP] Temporary process priority boost enabled duration_ms={}",
        STARTUP_BOOST_DURATION.as_millis()
    );

    let _ = std::thread::Builder::new()
        .name("startup-priority-restore".to_string())
        .spawn(move || {
            std::thread::sleep(STARTUP_BOOST_DURATION);
            let process = unsafe { GetCurrentProcess() };
            unsafe {
                if let Err(error) =
                    SetPriorityClass(process, PROCESS_CREATION_FLAGS(original_priority))
                {
                    log::warn!(
                        "[STARTUP] Failed to restore process priority after startup boost: {error}"
                    );
                } else {
                    log::info!("[STARTUP] Temporary process priority boost restored");
                }
            }
        });
}

#[cfg(not(target_os = "windows"))]
fn start_temporary_startup_priority_boost() {}

fn main() -> eframe::Result<()> {
    let startup_start = Instant::now();
    // SEC: Remove the current working directory from the default DLL search order.
    // Prevents DLL planting attacks (e.g. malicious pdfium.dll or libmpv-2.dll in CWD).
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::LibraryLoader::SetDefaultDllDirectories;
        use windows::Win32::System::LibraryLoader::LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;
        if let Err(error) = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) {
            log::warn!(
                "DLL search hardening failed: {} (process continues with reduced hardening)",
                error
            );
        }
    }

    // When running without a console (installed binary launched from shortcut),
    // reduce the default log level. Background worker threads continuously emit
    // log::info! which formats a String and acquires the global Stderr mutex.
    // With no real console the write fails instantly, so workers cycle through
    // alloc→lock→fail→unlock→free extremely fast, creating heavy heap-allocator
    // contention with the UI thread and causing scroll stutter.
    // Raising the floor to `warn` eliminates the vast majority of those
    // allocations while preserving actionable diagnostics.
    #[cfg(target_os = "windows")]
    let default_filter = if has_stderr_console() {
        "warn,wgpu_hal::vulkan::conv=error,mtt_file_manager=info"
    } else {
        "warn,wgpu_hal::vulkan::conv=error,mtt_file_manager=warn"
    };
    #[cfg(not(target_os = "windows"))]
    let default_filter = "warn,mtt_file_manager=info";

    let mut log_builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter));
    log_builder.format_timestamp_millis();
    let console_logger = log_builder.build();
    mtt_file_manager::infrastructure::diagnostic_logger::init(console_logger)
        .expect("global logger should initialize exactly once");
    log::info!(
        "[STARTUP] logger initialized elapsed_ms={}",
        startup_start.elapsed().as_millis()
    );

    let early_prefs_start = Instant::now();
    let early_prefs = read_early_preferences(&[
        mtt_file_manager::infrastructure::diagnostic_logger::DIAGNOSTIC_MODE_KEY,
        mtt_file_manager::infrastructure::diagnostic_logger::DIAGNOSTIC_MODE_ENABLED_AT_KEY,
        "gpu_backend",
    ]);
    log::info!(
        "[STARTUP] early preferences loaded count={} elapsed_ms={}",
        early_prefs.len(),
        early_prefs_start.elapsed().as_millis()
    );

    let diagnostic_mode_requested = early_prefs
        .get(mtt_file_manager::infrastructure::diagnostic_logger::DIAGNOSTIC_MODE_KEY)
        .map(String::as_str)
        .map(|value| value == "true")
        .unwrap_or(false);
    let diagnostic_enabled_at =
        mtt_file_manager::infrastructure::diagnostic_logger::parse_enabled_at_preference(
            early_prefs
                .get(mtt_file_manager::infrastructure::diagnostic_logger::DIAGNOSTIC_MODE_ENABLED_AT_KEY)
                .map(String::as_str),
        )
        .or_else(|| diagnostic_mode_requested.then_some(SystemTime::now()));
    let diagnostic_mode_active = diagnostic_mode_requested
        && !mtt_file_manager::infrastructure::diagnostic_logger::is_preference_expired(
            diagnostic_enabled_at,
            SystemTime::now(),
        );
    if diagnostic_mode_active {
        if let Some(enabled_since) = diagnostic_enabled_at {
            match mtt_file_manager::infrastructure::diagnostic_logger::enable_file_logging_with_since(
                enabled_since,
            ) {
                Ok(_) => {
                    log::info!("[DIAGNOSTIC] Diagnostic mode active during startup");
                    mtt_file_manager::infrastructure::diagnostic_logger::diag_info(
                        "startup",
                        "diagnostic_mode_restored",
                        &[mtt_file_manager::infrastructure::diagnostic_logger::field_label(
                            "source",
                            "startup_preference",
                        )],
                    );
                }
                Err(error) => {
                    log::error!(
                        "[DIAGNOSTIC] Failed to activate diagnostic file logging during startup: {}",
                        error
                    );
                }
            }
        }
    }

    // Standalone dedicated image viewer mode (separate process).
    let mut args = std::env::args_os();
    let _exe = args.next();
    if let Some(flag) = args.next() {
        let flag_str = flag.to_string_lossy();
        if flag_str.eq_ignore_ascii_case("--set-volume-label") {
            let Some(drive_path) = args.next() else {
                log::error!("[VOLUME-RENAME] missing drive path argument");
                std::process::exit(2);
            };
            let Some(new_label) = args.next() else {
                log::error!("[VOLUME-RENAME] missing label argument");
                std::process::exit(2);
            };

            let exit_code =
                mtt_file_manager::infrastructure::windows::run_elevated_volume_rename_helper(
                    &PathBuf::from(drive_path),
                    &new_label.to_string_lossy(),
                );
            std::process::exit(exit_code);
        }
        if flag_str.eq_ignore_ascii_case("--image-viewer") {
            if let Some(path_arg) = args.next() {
                return mtt_file_manager::image_viewer::run_standalone(PathBuf::from(path_arg));
            }

            log::error!("[IMAGE-VIEWER] missing path argument for --image-viewer");
            return Ok(());
        }
        if flag_str.eq_ignore_ascii_case("--pdf-viewer") {
            if let Some(path_arg) = args.next() {
                return mtt_file_manager::pdf_viewer::run_standalone(PathBuf::from(path_arg));
            }

            log::error!("[PDF-VIEWER] missing path argument for --pdf-viewer");
            return Ok(());
        }
        if flag_str.eq_ignore_ascii_case("--text-viewer") {
            if let Some(path_arg) = args.next() {
                return mtt_file_manager::text_viewer::run_standalone(PathBuf::from(path_arg));
            }

            log::error!("[TEXT-VIEWER] missing path argument for --text-viewer");
            return Ok(());
        }
        if flag_str.eq_ignore_ascii_case("--video-player") {
            if let Some(path_arg) = args.next() {
                let mut position: f64 = 0.0;
                let mut volume: f32 = 1.0;
                // Parse optional --position and --volume args
                while let Some(opt) = args.next() {
                    let opt_str = opt.to_string_lossy().to_string();
                    if opt_str.eq_ignore_ascii_case("--position") {
                        if let Some(val) = args.next() {
                            position = val.to_string_lossy().parse().unwrap_or(0.0);
                        }
                    } else if opt_str.eq_ignore_ascii_case("--volume") {
                        if let Some(val) = args.next() {
                            volume = val.to_string_lossy().parse().unwrap_or(1.0);
                        }
                    }
                }
                return mtt_file_manager::video_player::run_standalone(
                    PathBuf::from(path_arg),
                    position,
                    volume,
                );
            }

            log::error!("[VIDEO-PLAYER] missing path argument for --video-player");
            return Ok(());
        }
    }

    log::info!("MTT File Manager starting");
    start_temporary_startup_priority_boost();

    // Initialize codec name cache (queries Windows Registry on-demand)
    mtt_file_manager::infrastructure::windows::codec_registry::init_codec_cache();

    // Load application icon
    let icon_start = Instant::now();
    let icon_data = load_app_icon();
    log::info!(
        "[STARTUP] app icon loaded present={} elapsed_ms={}",
        icon_data.is_some(),
        icon_start.elapsed().as_millis()
    );

    // We manage preferences ourselves and force-kill on exit to avoid the
    // OneDrive/cldflt zombie-process hang. That makes eframe's RON storage
    // both unnecessary and prone to truncation, so clear it before startup.
    cleanup_eframe_storage(APP_ID);

    // 3-STAGE STARTUP: Start hidden and small (NOT maximized here)
    let mut viewport = egui::ViewportBuilder::default()
        .with_visible(false) // Start hidden
        .with_maximized(false) // NOT maximized at creation
        .with_inner_size([800.0, 600.0]) // Small initial size (will be maximized in update)
        .with_min_inner_size([800.0, 520.0])
        .with_title("MTT File Manager")
        .with_app_id(APP_ID)
        .with_decorations(false) // Borderless window - resize handled by native subclass
        .with_resizable(true); // Enable resize (handled by WM_NCHITTEST subclass)

    // Set window icon if loaded successfully
    if let Some(icon) = icon_data {
        viewport = viewport.with_icon(icon);
    }

    // Read user's GPU backend preference (before eframe init).
    let gpu_backend_pref =
        early_prefs
            .get("gpu_backend")
            .cloned()
            .map(|pref| match pref.as_str() {
                "vulkan" => "auto".to_string(),
                "gl" => "glow".to_string(),
                _ => pref,
            });
    let use_glow = match gpu_backend_pref.as_deref() {
        Some("glow") => true,
        _ => false, // default/auto: Wgpu with Vulkan priority on Windows
    };

    let options = if use_glow {
        log::info!("[STARTUP] Using Glow (OpenGL) renderer");
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Glow,
            persist_window: false,
            ..Default::default()
        }
    } else {
        let selected_backends =
            gpu_backend::parse_gpu_backend_preference(gpu_backend_pref.as_deref());
        let native_adapter_selector = gpu_backend::adapter_selector(gpu_backend_pref.as_deref());
        log::info!(
            "[STARTUP] Using Wgpu renderer. Backend preference: {:?} -> backends: {:?}",
            gpu_backend_pref.as_deref().unwrap_or("auto"),
            selected_backends
        );
        eframe::NativeOptions {
            viewport,
            renderer: eframe::Renderer::Wgpu,
            persist_window: false,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                    eframe::egui_wgpu::WgpuSetupCreateNew {
                        instance_descriptor: eframe::wgpu::InstanceDescriptor {
                            backends: selected_backends,
                            ..Default::default()
                        },
                        power_preference: eframe::wgpu::PowerPreference::HighPerformance,
                        native_adapter_selector,
                        device_descriptor: std::sync::Arc::new(|_adapter| {
                            eframe::wgpu::DeviceDescriptor {
                                label: Some("mtt-file-manager wgpu device"),
                                required_features: eframe::wgpu::Features::default(),
                                required_limits: eframe::wgpu::Limits {
                                    max_texture_dimension_2d:
                                        gpu_backend::WGPU_REQUIRED_MAX_TEXTURE_DIMENSION_2D,
                                    ..eframe::wgpu::Limits::default()
                                },
                                memory_hints: eframe::wgpu::MemoryHints::MemoryUsage,
                            }
                        }),
                        ..Default::default()
                    },
                ),
                desired_maximum_frame_latency: Some(1),
                ..Default::default()
            },
            ..Default::default()
        }
    };

    log::info!(
        "[STARTUP] native options ready elapsed_ms={}",
        startup_start.elapsed().as_millis()
    );

    let result = eframe::run_native(
        "MTT File Manager",
        options,
        Box::new(|cc| {
            // STARTUP OPTIMIZATION: Fonts are now loaded asynchronously in app/init.rs
            // This allows the window to appear immediately with default fonts.
            // When the background thread finishes, the new fonts (Segoe UI) are applied dynamically.
            let app_new_start = Instant::now();
            let app = ImageViewerApp::new(cc);
            log::info!(
                "[STARTUP] ImageViewerApp::new elapsed_ms={}",
                app_new_start.elapsed().as_millis()
            );

            Ok(Box::new(app))
        }),
    );

    // Belt-and-suspenders: if eframe returned (window closed) but the process
    // is still alive (background threads stuck in kernel), force-kill immediately.
    // handle_exit() normally calls std::process::exit(0), so this path is only
    // reached if something bypassed it.
    #[cfg(target_os = "windows")]
    {
        let _ = std::thread::spawn(
            mtt_file_manager::infrastructure::windows::cancel_pending_io_on_current_process_threads,
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
        mtt_file_manager::infrastructure::windows::terminate_current_process(0);
    }

    result
}
