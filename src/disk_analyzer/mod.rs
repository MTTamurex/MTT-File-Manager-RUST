//! Standalone disk usage analyzer process (`--disk-analyzer <LETTER>`).
//!
//! Runs as its own OS process — same model as the dedicated image/PDF/text
//! viewers — so the analyzer window gets an independent taskbar button and
//! minimize/restore lifecycle instead of being tied to the main window.

pub mod open_in_main;

use crate::app::disk_analysis_state::{AnalyzerDriveSummary, DiskAnalysisState};
use crate::viewer_runtime;
use eframe::egui;

/// Entry point for the `--disk-analyzer` subprocess.
pub fn run_standalone(drive_letter: char) -> eframe::Result<()> {
    // Capture panics so we can diagnose shutdown crashes even when stderr
    // is not attached (GUI-subsystem binary launched from shortcut).
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("[DISK-ANALYZER] PANIC: {}", info);
        default_panic(info);
    }));

    log::info!(
        "[DISK-ANALYZER] run_standalone enter pid={} drive={drive_letter}:",
        std::process::id()
    );

    viewer_runtime::apply_saved_locale();

    // Remove any stale eframe storage (app.ron) written by previous runs.
    // eframe restores persisted window state (position, size, visibility)
    // before with_visible(false) takes effect, causing startup flicker.
    if let Some(mut p) = dirs::data_dir() {
        p.push("mtt-file-manager-disk-analyzer");
        p.push("data");
        p.push("app.ron");
        let _ = std::fs::remove_file(&p);
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(rust_i18n::t!("disk_analysis.title").to_string())
        .with_inner_size([1150.0, 720.0])
        .with_min_inner_size([760.0, 480.0])
        .with_visible(false)
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-disk-analyzer");

    if let Ok(img) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
        let rgba_image = resized.to_rgba8();
        viewport = viewport.with_icon(egui::IconData {
            rgba: rgba_image.into_raw(),
            width: 256,
            height: 256,
        });
    }

    let native_options = viewer_runtime::build_viewer_native_options(viewport);
    let dark_mode = viewer_runtime::is_saved_theme_dark();

    let result = eframe::run_native(
        &rust_i18n::t!("disk_analysis.title"),
        native_options,
        Box::new(move |_cc| Ok(Box::new(DiskAnalyzerApp::new(drive_letter, dark_mode)))),
    );

    // Force-exit to avoid hangs from detached background threads
    // (the analyzer worker blocked on its request channel, etc.).
    // The viewers use the same belt-and-suspenders approach.
    #[cfg(target_os = "windows")]
    {
        let _ = std::thread::spawn(
            crate::infrastructure::windows::cancel_pending_io_on_current_process_threads,
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
        crate::infrastructure::windows::terminate_current_process(0);
    }

    result
}

struct DiskAnalyzerApp {
    state: DiskAnalysisState,
    dark_mode: bool,
    revealed: bool,
    last_theme_poll: std::time::Instant,
}

impl DiskAnalyzerApp {
    fn new(drive_letter: char, dark_mode: bool) -> Self {
        let mut state = DiskAnalysisState::new();
        state.drives = collect_drive_summaries();
        state.request(drive_letter);
        Self {
            state,
            dark_mode,
            revealed: false,
            last_theme_poll: std::time::Instant::now(),
        }
    }
}

impl eframe::App for DiskAnalyzerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.revealed {
            // Apply the saved theme before the first visible frame.
            if self.dark_mode {
                ctx.set_visuals(egui::Visuals::dark());
            } else {
                ctx.set_visuals(egui::Visuals::light());
            }
            crate::ui::theme::apply_scroll_style(&ctx);
            crate::ui::theme::apply_popup_style(&ctx);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.revealed = true;
        }

        // Follow main-app theme switches while the window is open.
        if let Some(dark) = crate::viewer_runtime::poll_saved_theme_change(
            self.dark_mode,
            &mut self.last_theme_poll,
        ) {
            self.dark_mode = dark;
            if dark {
                ctx.set_visuals(egui::Visuals::dark());
            } else {
                ctx.set_visuals(egui::Visuals::light());
            }
            crate::ui::theme::apply_scroll_style(&ctx);
            crate::ui::theme::apply_popup_style(&ctx);
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Escape closes the context menu first, then the window.
            if self.state.context_menu.is_some() {
                self.state.context_menu = None;
                egui::Popup::close_all(&ctx);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        crate::ui::disk_analysis::render_analyzer_body(&mut self.state, ui);
    }
}

/// Local drive facts for the sidebar/header (this process has no DriveState).
fn collect_drive_summaries() -> Vec<AnalyzerDriveSummary> {
    let (disks, _unavailable_label_roots) =
        crate::infrastructure::windows::get_all_drives_with_label_status();
    let mut drives = Vec::new();
    for (path, label) in disks {
        let Some(letter) = path.chars().next().filter(|c| c.is_ascii_alphabetic()) else {
            continue;
        };
        let vol = crate::infrastructure::windows::get_volume_info(&path);
        drives.push(AnalyzerDriveSummary {
            letter: letter.to_ascii_uppercase(),
            label,
            file_system: vol.file_system,
            total_space: vol.total_space,
            free_space: vol.free_space,
        });
    }
    drives
}
