use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use betteroffice_xlsx::Viewport;
use eframe::egui;
use rust_i18n::t;

use super::render_worker::{WorkerEvent, XlsxRenderWorker};
use super::renderer::{XlsxSearchMatch, XlsxViewportTile};

mod rendering;
mod search;
mod selection;
mod toolbar;

use selection::{DragSelection, SheetSelection};

#[derive(Clone, Copy, PartialEq)]
pub(super) struct RenderKey {
    pub sheet_index: usize,
    pub viewport: Viewport,
    pub zoom: f32,
}

struct RenderedTextureTile {
    texture: egui::TextureHandle,
    viewport: Viewport,
    body_offset_x: f32,
    body_offset_y: f32,
    frozen_width: f32,
    frozen_height: f32,
}

struct RenderedTexture {
    tiles: Vec<RenderedTextureTile>,
    key: RenderKey,
}

enum ViewerStatus {
    Opening,
    Ready,
    Failed(String),
}

pub(super) struct XlsxViewerApp {
    file_path: PathBuf,
    worker: Option<XlsxRenderWorker>,
    status: ViewerStatus,
    sheet_names: Vec<String>,
    selected_sheet: usize,
    sheet_info: Option<betteroffice_xlsx::SheetInfo>,
    texture: Option<RenderedTexture>,
    render_error: Option<(RenderKey, String)>,
    pending_render: Option<u64>,
    next_render_generation: u64,
    latest_render_key: Option<RenderKey>,
    scroll_to_position: Option<(f32, f32)>,
    pending_search_navigation: Option<(usize, (f32, f32))>,
    search_active: bool,
    search_query: String,
    search_input_focus_requested: bool,
    search_input_has_focus: bool,
    search_results: Vec<XlsxSearchMatch>,
    current_match_idx: usize,
    search_generation: u64,
    search_in_progress: bool,
    last_searched_query: String,
    zoom: f32,
    dark_mode: Option<bool>,
    native_hwnd: Option<isize>,
    last_theme_poll: Instant,
    last_memory_activity: Instant,
    last_memory_trim_check: Instant,
    last_memory_trim_request: Instant,
    memory_trim_was_pending: bool,
    memory_activity_generation: Arc<AtomicU64>,
    drag_selection: Option<DragSelection>,
    selection: Option<SheetSelection>,
    selection_generation: u64,
}

impl XlsxViewerApp {
    pub fn new(path: PathBuf, dark_mode: bool) -> Self {
        let now = Instant::now();
        Self {
            file_path: path,
            worker: None,
            status: ViewerStatus::Opening,
            sheet_names: Vec::new(),
            selected_sheet: 0,
            sheet_info: None,
            texture: None,
            render_error: None,
            pending_render: None,
            next_render_generation: 0,
            latest_render_key: None,
            scroll_to_position: None,
            pending_search_navigation: None,
            search_active: false,
            search_query: String::new(),
            search_input_focus_requested: false,
            search_input_has_focus: false,
            search_results: Vec::new(),
            current_match_idx: 0,
            search_generation: 0,
            search_in_progress: false,
            last_searched_query: String::new(),
            zoom: 1.0,
            dark_mode: Some(dark_mode),
            native_hwnd: None,
            last_theme_poll: now,
            last_memory_activity: now,
            last_memory_trim_check: now,
            last_memory_trim_request: now.checked_sub(Duration::from_secs(10)).unwrap_or(now),
            memory_trim_was_pending: true,
            memory_activity_generation: Arc::new(AtomicU64::new(0)),
            drag_selection: None,
            selection: None,
            selection_generation: 0,
        }
    }

    fn ensure_worker(&mut self, ctx: &egui::Context) {
        if self.worker.is_none() && matches!(self.status, ViewerStatus::Opening) {
            self.worker = Some(XlsxRenderWorker::spawn(self.file_path.clone(), ctx.clone()));
        }
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let events = self
            .worker
            .as_ref()
            .map(XlsxRenderWorker::drain_events)
            .unwrap_or_default();
        for event in events {
            match event {
                WorkerEvent::Opened(opened) => {
                    let initial_scroll = (
                        opened.sheet_info.initial_scroll_x,
                        opened.sheet_info.initial_scroll_y,
                    );
                    self.sheet_names = opened.sheet_info.sheet_names.clone();
                    self.selected_sheet = opened.sheet_info.active_sheet.0 as usize;
                    self.sheet_info = Some(opened.sheet_info);
                    self.scroll_to_position = Some(initial_scroll);
                    self.status = ViewerStatus::Ready;
                }
                WorkerEvent::SheetSelected {
                    sheet_index,
                    sheet_info,
                } => {
                    let initial_scroll = (sheet_info.initial_scroll_x, sheet_info.initial_scroll_y);
                    self.selected_sheet = sheet_index;
                    self.sheet_info = Some(sheet_info);
                    self.texture = None;
                    self.render_error = None;
                    self.scroll_to_position = Some(initial_scroll);
                    self.apply_pending_search_navigation(sheet_index);
                }
                WorkerEvent::Rendered {
                    generation,
                    key,
                    pixels,
                } => {
                    self.finish_render_request(generation);
                    if self.latest_render_key == Some(key) {
                        self.accept_rendered_pixels(ctx, key, pixels);
                    }
                }
                WorkerEvent::RenderFailed {
                    generation,
                    key,
                    error,
                } => {
                    self.finish_render_request(generation);
                    if self.latest_render_key == Some(key) {
                        self.render_error = Some((key, error));
                    }
                }
                WorkerEvent::SearchCompleted {
                    generation,
                    query,
                    matches,
                } => self.accept_search_result(generation, query, matches),
                WorkerEvent::RangeSelected {
                    generation,
                    sheet_index,
                    result,
                } => self.accept_range_selection(generation, sheet_index, result),
                WorkerEvent::Failed(error) => self.status = ViewerStatus::Failed(error),
            }
        }
    }

    fn accept_rendered_pixels(
        &mut self,
        ctx: &egui::Context,
        key: RenderKey,
        pixels: Vec<XlsxViewportTile>,
    ) {
        let tiles = pixels
            .into_iter()
            .enumerate()
            .map(|(index, tile)| {
                let color_image =
                    egui::ColorImage::from_rgba_unmultiplied([tile.width, tile.height], &tile.rgba);
                RenderedTextureTile {
                    texture: ctx.load_texture(
                        format!("xlsx_viewport_{index}"),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ),
                    viewport: tile.viewport,
                    body_offset_x: tile.body_offset_x,
                    body_offset_y: tile.body_offset_y,
                    frozen_width: tile.frozen_width,
                    frozen_height: tile.frozen_height,
                }
            })
            .collect();
        self.texture = Some(RenderedTexture { tiles, key });
        self.render_error = None;
    }

    fn finish_render_request(&mut self, generation: u64) {
        if self.pending_render == Some(generation) {
            self.pending_render = None;
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
            || (matches!(self.status, ViewerStatus::Ready) && self.sheet_info.is_none())
            || self.search_in_progress
            || self.pending_render.is_some()
            || self.pending_search_navigation.is_some()
            || self
                .worker
                .as_ref()
                .is_some_and(XlsxRenderWorker::has_pending_results)
    }

    fn run_idle_working_set_trim(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if ctx.input(|input| !input.raw.events.is_empty()) {
            self.last_memory_activity = now;
            self.memory_activity_generation
                .fetch_add(1, Ordering::AcqRel);
        }

        let has_pending_work = self.memory_trim_has_pending_work();
        if has_pending_work {
            if !self.memory_trim_was_pending {
                self.memory_activity_generation
                    .fetch_add(1, Ordering::AcqRel);
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

        let generation = self.memory_activity_generation.load(Ordering::Acquire);
        if crate::image_viewer::metrics::request_idle_working_set_trim(
            Arc::clone(&self.memory_activity_generation),
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

    pub(super) fn select_sheet(&mut self, sheet_index: usize) {
        if sheet_index == self.selected_sheet || sheet_index >= self.sheet_names.len() {
            return;
        }
        self.selection_generation = self.selection_generation.wrapping_add(1);
        self.drag_selection = None;
        self.selection = None;
        let requested = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.request_sheet(sheet_index));
        if requested {
            self.selected_sheet = sheet_index;
            self.sheet_info = None;
            self.texture = None;
            self.render_error = None;
            self.latest_render_key = None;
        }
    }

    pub(super) fn select_sheet_for_search(&mut self, match_result: &XlsxSearchMatch) {
        if match_result.sheet_index == self.selected_sheet {
            self.scroll_to_position = Some(match_result.scroll_position);
        } else {
            self.pending_search_navigation =
                Some((match_result.sheet_index, match_result.scroll_position));
            self.select_sheet(match_result.sheet_index);
        }
    }

    fn apply_pending_search_navigation(&mut self, sheet_index: usize) {
        if let Some((pending_sheet, position)) = self.pending_search_navigation {
            if pending_sheet == sheet_index {
                self.scroll_to_position = Some(position);
                self.pending_search_navigation = None;
            }
        }
    }

    fn accept_range_selection(
        &mut self,
        generation: u64,
        sheet_index: usize,
        result: Result<super::cell_selection::CellSelection, String>,
    ) {
        if generation != self.selection_generation || sheet_index != self.selected_sheet {
            return;
        }
        match result {
            Ok(selection) => self.selection = Some(SheetSelection::from(selection)),
            Err(error) => log::warn!("[XLSX-VIEWER] could not copy selected range: {error}"),
        }
    }
}

impl eframe::App for XlsxViewerApp {
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
                        ui.label(t!("xlsxviewer.loading_workbook").to_string());
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

        self.handle_search_shortcuts(&ctx);
        self.handle_selection_shortcuts(&ctx);
        self.handle_keyboard(&ctx);
        self.show_toolbar_frames(ui);
        self.show_search_bar(ui);
        egui::CentralPanel::default().show(ui, |ui| self.show_sheet(ui));
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

#[cfg(test)]
mod tests {
    use super::{ViewerStatus, XlsxViewerApp};
    use betteroffice_xlsx::{SheetId, SheetInfo};
    use std::path::PathBuf;

    fn ready_app() -> XlsxViewerApp {
        let mut app = XlsxViewerApp::new(PathBuf::new(), false);
        app.status = ViewerStatus::Ready;
        app.sheet_info = Some(SheetInfo {
            sheet_ids: vec!["sheet:0".to_owned()],
            sheet_names: vec!["Sheet".to_owned()],
            active_sheet: SheetId(0),
            content_width: 640.0,
            content_height: 480.0,
            frozen_rows: 0,
            frozen_cols: 0,
            initial_scroll_x: 0.0,
            initial_scroll_y: 0.0,
        });
        app
    }

    #[test]
    fn memory_trim_waits_for_viewer_work_to_finish() {
        let mut app = ready_app();
        assert!(!app.memory_trim_has_pending_work());

        app.status = ViewerStatus::Opening;
        assert!(app.memory_trim_has_pending_work());
        app.status = ViewerStatus::Ready;

        app.sheet_info = None;
        assert!(app.memory_trim_has_pending_work());
        app.sheet_info = ready_app().sheet_info;

        app.pending_render = Some(1);
        assert!(app.memory_trim_has_pending_work());
        app.pending_render = None;

        app.search_in_progress = true;
        assert!(app.memory_trim_has_pending_work());
        app.search_in_progress = false;

        app.pending_search_navigation = Some((0, (0.0, 0.0)));
        assert!(app.memory_trim_has_pending_work());
    }
}
