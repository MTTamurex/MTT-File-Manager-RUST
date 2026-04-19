//! Shared lightweight runtime helpers for the standalone viewer subprocesses
//! (`--image-viewer`, `--pdf-viewer`, `--text-viewer`).
//!
//! The viewers run as separate processes spawned from the same binary as the
//! main file manager, so without care each one would inherit the file
//! manager's heavy startup cost (full SQLite ORM init, all wgpu backends,
//! discrete-GPU device, performance memory hints, etc.) just to display a
//! single file. This module concentrates the minimal-baseline configuration:
//!
//! * Read user prefs (locale, theme) with a tiny read-only SQLite query
//!   instead of the full [`crate::infrastructure::app_state_db::AppStateDb`]
//!   pipeline (which runs migrations and pragmas on every open).
//! * Build [`eframe::NativeOptions`] that
//!   - prefer the **integrated GPU** (`PowerPreference::LowPower`) — viewers
//!     do not need the discrete GPU's compute throughput, and the iGPU keeps
//!     the working set drastically smaller on hybrid laptops;
//!   - on Windows, restrict the wgpu instance to the **DX12 backend only**
//!     (skipping Vulkan / GL / DX11 driver loading);
//!   - request `MemoryHints::MemoryUsage` from wgpu when creating the
//!     device, telling the driver to favour smaller staging / pool sizes;
//!   - cap `max_texture_dimension_2d` to 4096 px (4K monitors are ≤ 3840 px
//!     wide, so this still covers a maximised window on a single 4K display
//!     and avoids wgpu reserving headroom for 8K surfaces);
//!   - disable optional GL-only buffers (`depth_buffer`, `stencil_buffer`,
//!     `multisampling`) which the viewers never use.

use eframe::egui;
use std::path::PathBuf;
use std::sync::Arc;

fn state_db_path() -> Option<PathBuf> {
    Some(
        dirs::data_local_dir()?
            .join("MTT-File-Manager")
            .join("state")
            .join("app_state.db"),
    )
}

/// Reads a single value from the `user_preferences` table using a lightweight
/// read-only SQLite connection. Avoids running the full
/// [`crate::infrastructure::app_state_db::AppStateDb`] init (migrations,
/// pragmas, prepared-statement cache) which on cold start can cost tens of
/// MB of resident memory in each viewer process.
fn read_pref_readonly(key: &str) -> Option<String> {
    let db_path = state_db_path()?;
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM user_preferences WHERE key = ?",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .ok();
    // Connection drops here, releasing any cached pages.
    value
}

/// Apply the saved UI language to `rust_i18n` for this viewer process.
pub fn apply_saved_locale() {
    if let Some(language) = read_pref_readonly("language") {
        rust_i18n::set_locale(&language);
    }
}

/// Returns `true` if the user's saved theme is "dark", `false` otherwise
/// (including when no preference is stored).
pub fn is_saved_theme_dark() -> bool {
    read_pref_readonly("theme_mode")
        .map(|s| s == "dark")
        .unwrap_or(false)
}

/// Build [`eframe::NativeOptions`] tuned for a low-baseline-RAM viewer
/// subprocess. See the module-level docs for the rationale of each knob.
pub fn build_viewer_native_options(viewport: egui::ViewportBuilder) -> eframe::NativeOptions {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;

    // Custom device descriptor: ask the driver to minimise pool allocations
    // and cap the surface texture size to 4 K.
    let device_descriptor: Arc<dyn Fn(&wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> + Send + Sync> =
        Arc::new(|adapter| {
            let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            };

            wgpu::DeviceDescriptor {
                label: Some("mtt-viewer wgpu device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits {
                    // 4096 covers a maximised window on a single 4K monitor
                    // while letting the driver allocate smaller texture
                    // pools than the 8192 default would force.
                    max_texture_dimension_2d: 4096,
                    ..base_limits
                },
                memory_hints: wgpu::MemoryHints::MemoryUsage,
            }
        });

    // Restrict the wgpu Instance to a single backend so we don't pay for
    // loading Vulkan / OpenGL drivers on Windows just to render egui.
    #[cfg(target_os = "windows")]
    let instance_backends = wgpu::Backends::DX12;
    #[cfg(not(target_os = "windows"))]
    let instance_backends = wgpu::Backends::PRIMARY;

    let setup = WgpuSetupCreateNew {
        instance_descriptor: wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(instance_backends),
            flags: wgpu::InstanceFlags::from_build_config().with_env(),
            backend_options: wgpu::BackendOptions::from_env_or_default(),
        },
        // LowPower hints the driver to use the integrated GPU on hybrid
        // systems. Viewers don't need the dGPU's throughput and the iGPU
        // keeps the resident set far smaller (no separate VRAM mirror).
        power_preference: wgpu::PowerPreference::from_env()
            .unwrap_or(wgpu::PowerPreference::LowPower),
        native_adapter_selector: None,
        device_descriptor,
        trace_path: std::env::var("WGPU_TRACE")
            .ok()
            .map(std::path::PathBuf::from),
    };

    eframe::NativeOptions {
        viewport,
        // No persisted egui state — viewers are ephemeral and we already
        // call cleanup_eframe_storage() in the file-manager entrypoint.
        persist_window: false,
        // Disable optional buffers the viewers never use; this keeps the
        // surface allocation as small as possible (mostly relevant for the
        // glow renderer, harmless under wgpu).
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: WgpuSetup::CreateNew(setup),
            // 1 frame in flight is enough for a viewer (no animation
            // pipelines beyond GIF playback) and avoids the driver
            // double-/triple-buffering large frame resources.
            desired_maximum_frame_latency: Some(1),
            ..Default::default()
        },
        ..Default::default()
    }
}
