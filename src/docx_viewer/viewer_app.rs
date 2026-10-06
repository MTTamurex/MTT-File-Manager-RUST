use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use rust_i18n::t;

use super::render_worker::{DocxRenderWorker, WorkerEvent};
mod rendering;
mod toolbar;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ZoomMode {
    FitWidth,
    FitPage,
    Custom,
}

enum ViewerStatus {
    Opening,
    Ready,
    Failed(String),
}

pub(super) struct PageTexture {
    texture: egui::TextureHandle,
    bytes: usize,
}

#[derive(Clone, Copy)]
pub(super) struct PageGeometry {
    rect: egui::Rect,
    size: egui::Vec2,
}

pub(super) struct DocxViewerApp {
    file_path: PathBuf,
    worker: Option<DocxRenderWorker>,
    status: ViewerStatus,
    page_sizes: Vec<(f32, f32)>,
    textures: HashMap<usize, PageTexture>,
    pending: HashSet<usize>,
    failed_pages: HashMap<usize, String>,
    cache_bytes: usize,
    current_page: usize,
    scroll_to_page: Option<usize>,
    zoom_mode: ZoomMode,
    zoom: f32,
    effective_zoom: f32,
    dark_mode: Option<bool>,
    native_hwnd: Option<isize>,
    last_theme_poll: Instant,
    last_memory_activity: Instant,
    last_memory_trim_check: Instant,
    last_memory_trim_request: Instant,
    memory_trim_was_pending: bool,
    memory_activity_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    skipped_images: usize,
}

impl DocxViewerApp {
    pub fn new(path: PathBuf, dark_mode: bool) -> Self {
        let now = Instant::now();
        Self {
            file_path: path,
            worker: None,
            status: ViewerStatus::Opening,
            page_sizes: Vec::new(),
            textures: HashMap::new(),
            pending: HashSet::new(),
            failed_pages: HashMap::new(),
            cache_bytes: 0,
            current_page: 0,
            scroll_to_page: None,
            zoom_mode: ZoomMode::FitWidth,
            zoom: 1.0,
            effective_zoom: 1.0,
            dark_mode: Some(dark_mode),
            native_hwnd: None,
            last_theme_poll: now,
            last_memory_activity: now,
            last_memory_trim_check: now,
            last_memory_trim_request: now.checked_sub(Duration::from_secs(10)).unwrap_or(now),
            memory_trim_was_pending: true,
            memory_activity_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            skipped_images: 0,
        }
    }

    fn ensure_worker(&mut self, ctx: &egui::Context) {
        if self.worker.is_none() && matches!(self.status, ViewerStatus::Opening) {
            self.worker = Some(DocxRenderWorker::spawn(self.file_path.clone(), ctx.clone()));
        }
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let events = self
            .worker
            .as_ref()
            .map(DocxRenderWorker::drain_events)
            .unwrap_or_default();
        for event in events {
            match event {
                WorkerEvent::Opened { page_sizes } => {
                    self.page_sizes = page_sizes;
                    self.status = ViewerStatus::Ready;
                    self.current_page = 0;
                    self.scroll_to_page = Some(0);
                }
                WorkerEvent::Rendered(page) => {
                    self.pending.remove(&page.page_index);
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [page.width, page.height],
                        &page.rgba,
                    );
                    let texture = ctx.load_texture(
                        format!("docx_page_{}", page.page_index),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    let bytes = page.rgba.len();
                    if let Some(previous) = self
                        .textures
                        .insert(page.page_index, PageTexture { texture, bytes })
                    {
                        self.cache_bytes = self.cache_bytes.saturating_sub(previous.bytes);
                    }
                    self.cache_bytes = self.cache_bytes.saturating_add(bytes);
                    self.skipped_images = self.skipped_images.saturating_add(page.skipped_images);
                }
                WorkerEvent::PageFailed { page_index, error } => {
                    self.pending.remove(&page_index);
                    self.failed_pages.insert(page_index, error);
                }
                WorkerEvent::Failed(error) => {
                    self.status = ViewerStatus::Failed(error);
                }
            }
        }
    }

    fn start_page_request(&mut self, page_index: usize) {
        if self.textures.contains_key(&page_index)
            || self.pending.contains(&page_index)
            || self.failed_pages.contains_key(&page_index)
        {
            return;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.request_page(page_index))
        {
            self.pending.insert(page_index);
        }
    }

    fn poll_theme(&mut self, ctx: &egui::Context) {
        if let Some(dark) = crate::viewer_runtime::poll_saved_theme_change(
            ctx.global_style().visuals.dark_mode,
            &mut self.last_theme_poll,
        ) {
            crate::ui::theme::apply_viewer_visuals(ctx, dark);
            if let Some(raw) = self.native_hwnd {
                crate::infrastructure::windows::window_corners::apply_dark_title_bar(
                    windows::Win32::Foundation::HWND(raw as *mut _),
                    dark,
                );
            }
        }
        crate::viewer_runtime::schedule_saved_theme_poll(ctx, self.last_theme_poll);
    }

    fn memory_trim_has_pending_work(&self) -> bool {
        matches!(self.status, ViewerStatus::Opening)
            || !self.pending.is_empty()
            || self
                .worker
                .as_ref()
                .is_some_and(DocxRenderWorker::has_pending_results)
    }

    fn run_idle_working_set_trim(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if ctx.input(|input| !input.raw.events.is_empty()) {
            self.last_memory_activity = now;
            self.memory_activity_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }

        let has_pending_work = self.memory_trim_has_pending_work();
        if has_pending_work {
            if !self.memory_trim_was_pending {
                self.memory_activity_generation
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                self.last_memory_activity = now;
            }
            self.memory_trim_was_pending = true;
            ctx.request_repaint_after(Duration::from_secs(1));
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
        let cache_trim_requested = self
            .worker
            .as_ref()
            .is_some_and(DocxRenderWorker::request_cache_trim);
        let working_set_trim_requested =
            crate::image_viewer::metrics::request_idle_working_set_trim(
                std::sync::Arc::clone(&self.memory_activity_generation),
                generation,
            );
        if cache_trim_requested || working_set_trim_requested {
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

impl eframe::App for DocxViewerApp {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        crate::viewer_runtime::poll_viewer_fonts(ctx);
        if let Some(dark) = self.dark_mode.take() {
            crate::ui::theme::apply_viewer_visuals(ctx, dark);
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(wh) = handle.as_raw() {
                    let hwnd = windows::Win32::Foundation::HWND(wh.hwnd.get() as _);
                    self.native_hwnd = Some(wh.hwnd.get());
                    crate::infrastructure::windows::center_window_on_primary_monitor(hwnd);
                    crate::infrastructure::windows::window_corners::apply_dark_title_bar(
                        hwnd, dark,
                    );
                }
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.ensure_worker(ctx);
        self.poll_worker(ctx);
        self.run_idle_working_set_trim(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_theme(&ctx);
        ui.set_style(ctx.global_style());

        match &self.status {
            ViewerStatus::Opening => {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                        ui.label(t!("docxviewer.loading_document").to_string());
                    });
                });
                return;
            }
            ViewerStatus::Failed(error) => {
                let error = error.clone();
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            egui::RichText::new(error)
                                .color(egui::Color32::RED)
                                .size(16.0),
                        );
                    });
                });
                return;
            }
            ViewerStatus::Ready => {}
        }

        self.handle_keyboard(&ctx);
        self.show_toolbar_frame(ui);
        if self.skipped_images > 0 {
            egui::Panel::bottom("docx_status")
                .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 4)))
                .show(ui, |ui| {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        t!(
                            "docxviewer.skipped_images",
                            count = self.skipped_images.to_string()
                        )
                        .to_string(),
                    );
                });
        }
        egui::CentralPanel::default().show(ui, |ui| self.show_pages(ui));
    }
}

pub(super) struct ErrorApp {
    pub message: String,
}

impl eframe::App for ErrorApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::viewer_runtime::poll_viewer_fonts(ctx);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(&self.message)
                        .color(egui::Color32::RED)
                        .size(16.0),
                );
            });
        });
    }
}
