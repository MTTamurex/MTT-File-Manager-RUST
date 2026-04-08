#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

// Force the discrete GPU on hybrid-GPU laptops (NVIDIA Optimus / AMD PowerXpress).
// These exported symbols are read by the GPU driver at process start-up to decide
// which adapter to use.  Without them, the installed binary (windows_subsystem = "windows")
// may be routed to the integrated GPU, while `cargo run` from a VS Code terminal
// inherits the parent's GPU affinity and may get the discrete adapter by chance.
#[cfg(target_os = "windows")]
#[no_mangle]
pub static NvOptimusEnablement: u32 = 1;
#[cfg(target_os = "windows")]
#[no_mangle]
pub static AmdPowerXpressRequestHighPerformance: i32 = 1;

use eframe::egui;
use mtt_file_manager::app::ImageViewerApp;
use std::path::PathBuf;

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

/// Load application icon from embedded PNG bytes
fn load_app_icon() -> Option<egui::IconData> {
    // Load PNG from embedded bytes using image crate
    match image::load_from_memory(mtt_file_manager::embedded_assets::APP_ICON_PNG) {
        Ok(img) => {
            // Resize to 256x256 for optimal display (Windows icon standard)
            let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
            let rgba_image = resized.to_rgba8();
            let pixels = rgba_image.into_raw();

            Some(egui::IconData {
                rgba: pixels,
                width: 256,
                height: 256,
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

fn main() -> eframe::Result<()> {
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
        "warn,mtt_file_manager=info"
    } else {
        "warn,mtt_file_manager=warn"
    };
    #[cfg(not(target_os = "windows"))]
    let default_filter = "warn,mtt_file_manager=info";

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_filter))
        .format_timestamp_millis()
        .init();

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

            let exit_code = mtt_file_manager::infrastructure::windows::run_elevated_volume_rename_helper(
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

    // Initialize codec name cache (queries Windows Registry on-demand)
    mtt_file_manager::infrastructure::windows::codec_registry::init_codec_cache();

    // Load application icon
    let icon_data = load_app_icon();

    // We manage preferences ourselves and force-kill on exit to avoid the
    // OneDrive/cldflt zombie-process hang. That makes eframe's RON storage
    // both unnecessary and prone to truncation, so clear it before startup.
    cleanup_eframe_storage(APP_ID);

    // 3-STAGE STARTUP: Start hidden and small (NOT maximized here)
    let mut viewport = egui::ViewportBuilder::default()
        .with_visible(false) // Start hidden
        .with_maximized(false) // NOT maximized at creation
        .with_inner_size([800.0, 600.0]) // Small initial size (will be maximized in update)
        .with_title("MTT File Manager")
        .with_app_id(APP_ID)
        .with_decorations(false) // Borderless window - resize handled by native subclass
        .with_resizable(true); // Enable resize (handled by WM_NCHITTEST subclass)

    // Set window icon if loaded successfully
    if let Some(icon) = icon_data {
        viewport = viewport.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport,
        persist_window: false, // Disable eframe persistence - we control manually
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            // Request the discrete GPU on hybrid-GPU systems (e.g. laptop with
            // Intel + NVIDIA/AMD).  Without this, the driver may route the app
            // to the integrated GPU, causing slower texture uploads and lower
            // throughput — especially noticeable after returning from idle.
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(eframe::egui_wgpu::WgpuSetupCreateNew {
                power_preference: eframe::wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }),
            // Low-latency presentation: queue only 1 frame ahead so the
            // compositor shows our content sooner after texture re-uploads.
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
        ..Default::default()
    };

    let result = eframe::run_native(
        "MTT File Manager",
        options,
        Box::new(|cc| {
            // STARTUP OPTIMIZATION: Fonts are now loaded asynchronously in app/init.rs
            // This allows the window to appear immediately with default fonts.
            // When the background thread finishes, the new fonts (Segoe UI) are applied dynamically.

            Ok(Box::new(ImageViewerApp::new(cc)))
        }),
    );

    // Belt-and-suspenders: if eframe returned (window closed) but the process
    // is still alive (background threads stuck in kernel), allow a short grace
    // period before force-killing.  handle_exit() already performed cooperative
    // shutdown; this path is only reached if something bypassed it.
    #[cfg(target_os = "windows")]
    {
        let _ = std::thread::spawn(mtt_file_manager::ui::app::cancel_all_pending_io);
        std::thread::sleep(std::time::Duration::from_millis(100));
        unsafe {
            windows::Win32::System::Threading::TerminateProcess(
                windows::Win32::System::Threading::GetCurrentProcess(),
                0,
            ).ok();
        }
    }

    result
}
