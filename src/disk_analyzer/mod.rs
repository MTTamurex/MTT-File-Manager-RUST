//! Standalone disk usage analyzer process (`--disk-analyzer <LETTER>`).
//!
//! Runs as its own OS process — same model as the dedicated image/PDF/text
//! viewers — so the analyzer window gets an independent taskbar button and
//! minimize/restore lifecycle instead of being tied to the main window.

pub mod open_in_main;

use crate::app::disk_analysis_state::DiskAnalysisState;
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
    last_memory_activity: std::time::Instant,
    last_memory_trim_check: std::time::Instant,
    last_memory_trim_request: std::time::Instant,
    memory_trim_was_pending: bool,
    memory_activity_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl DiskAnalyzerApp {
    fn new(drive_letter: char, dark_mode: bool) -> Self {
        let mut state = DiskAnalysisState::new();
        state.drives = crate::app::disk_analysis_state::collect_drive_summaries();
        state.request(drive_letter);
        let now = std::time::Instant::now();
        Self {
            state,
            dark_mode,
            revealed: false,
            last_theme_poll: now,
            last_memory_activity: now,
            last_memory_trim_check: now,
            last_memory_trim_request: now
                .checked_sub(std::time::Duration::from_secs(10))
                .unwrap_or(now),
            memory_trim_was_pending: true,
            memory_activity_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn run_idle_working_set_trim(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        if ctx.input(|input| !input.raw.events.is_empty()) {
            self.last_memory_activity = now;
            self.memory_activity_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }

        let has_pending_work = self.state.memory_trim_has_pending_work();
        if has_pending_work {
            if !self.memory_trim_was_pending {
                self.memory_activity_generation
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                self.last_memory_activity = now;
            }
            self.memory_trim_was_pending = true;
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        } else if self.memory_trim_was_pending {
            self.memory_trim_was_pending = false;
            self.last_memory_activity = now;
        }

        let trim_check_interval = crate::image_viewer::metrics::working_set_trim_check_interval();
        let since_last_check = now.duration_since(self.last_memory_trim_check);
        if since_last_check < trim_check_interval {
            if !has_pending_work {
                ctx.request_repaint_after(trim_check_interval - since_last_check);
            }
            return;
        }
        self.last_memory_trim_check = now;

        let working_set_bytes = crate::image_viewer::metrics::current_working_set_bytes();
        if !crate::image_viewer::metrics::idle_trim_is_due(
            now,
            self.last_memory_activity,
            self.last_memory_trim_request,
            working_set_bytes,
            has_pending_work,
        ) {
            if !has_pending_work
                && working_set_bytes >= crate::image_viewer::metrics::working_set_trim_min_bytes()
            {
                let wait = crate::image_viewer::metrics::idle_trim_wait_remaining(
                    now,
                    self.last_memory_activity,
                    self.last_memory_trim_request,
                );
                ctx.request_repaint_after(wait.max(trim_check_interval));
            }
            return;
        }

        let generation = self
            .memory_activity_generation
            .load(std::sync::atomic::Ordering::Acquire);
        if crate::image_viewer::metrics::request_idle_working_set_trim(
            std::sync::Arc::clone(&self.memory_activity_generation),
            generation,
        ) {
            self.last_memory_trim_request = now;
            let wait = crate::image_viewer::metrics::idle_trim_wait_remaining(
                now,
                self.last_memory_activity,
                now,
            );
            ctx.request_repaint_after(wait.max(trim_check_interval));
        }
    }
}

impl eframe::App for DiskAnalyzerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Load system fonts on a background thread and apply once ready.
        crate::viewer_runtime::poll_viewer_fonts(&ctx);

        if !self.revealed {
            // Apply the saved theme before the first visible frame.
            crate::ui::theme::apply_viewer_visuals(&ctx, self.dark_mode);
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
            crate::ui::theme::apply_viewer_visuals(&ctx, dark);
            crate::ui::theme::apply_scroll_style(&ctx);
            crate::ui::theme::apply_popup_style(&ctx);
        }
        crate::viewer_runtime::schedule_saved_theme_poll(&ctx, self.last_theme_poll);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            // Escape closes the context menu first, then clears the search,
            // then the window (plan section 5).
            if self.state.context_menu.is_some() {
                self.state.context_menu = None;
                egui::Popup::close_all(&ctx);
            } else if self.state.search_open || !self.state.search_text.is_empty() {
                self.state.search_text.clear();
                self.state.mark_search_changed();
                self.state.search_results = std::sync::Arc::new(Vec::new());
                self.state.search_selected = 0;
                self.state.search_open = false;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }

        crate::ui::disk_analysis::render_analyzer_body(&mut self.state, ui);
        self.run_idle_working_set_trim(&ctx);
    }
}
