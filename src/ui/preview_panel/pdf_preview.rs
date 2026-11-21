use crate::domain::file_entry::FileEntry;
use crate::pdf_viewer::{render_preview_page, PdfPreviewPage};
use crate::ui::preview_panel::image_preview::render_texture_with_overlay;
use crate::ui::svg_icons::SvgIconManager;
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use std::path::PathBuf;
use std::sync::OnceLock;

const RENDER_WIDTH: u32 = 720;

struct PdfRequest {
    path: PathBuf,
    page: usize,
    reply: Sender<Result<PdfPreviewPage, String>>,
}

struct PdfWorker {
    sender: Sender<PdfRequest>,
    queue: Receiver<PdfRequest>,
}

static PDF_WORKER: OnceLock<Result<PdfWorker, String>> = OnceLock::new();

fn worker() -> Result<&'static PdfWorker, String> {
    PDF_WORKER
        .get_or_init(|| {
            let (sender, queue) = bounded::<PdfRequest>(1);
            let worker_queue = queue.clone();
            std::thread::Builder::new()
                .name("detail-pdf-preview".into())
                .spawn(move || {
                    while let Ok(request) = worker_queue.recv() {
                        let result = std::panic::catch_unwind(|| {
                            render_preview_page(&request.path, request.page, RENDER_WIDTH)
                        })
                        .unwrap_or_else(|_| Err("PDF preview worker panicked".into()));
                        let _ = request.reply.send(result);
                    }
                })
                .map_err(|error| error.to_string())?;
            Ok(PdfWorker { sender, queue })
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn request_page(
    path: PathBuf,
    page: usize,
) -> Result<Receiver<Result<PdfPreviewPage, String>>, String> {
    let (reply, result) = bounded(1);
    let request = PdfRequest { path, page, reply };
    let worker = worker()?;
    super::background::send_latest(&worker.sender, &worker.queue, request)
        .map_err(|_| "PDF preview worker stopped".to_string())?;
    Ok(result)
}

#[derive(Default, Clone)]
struct PdfPanelState {
    path: Option<PathBuf>,
    modified: u64,
    size: u64,
    page: usize,
    page_count: usize,
    texture: Option<egui::TextureHandle>,
    texture_page: Option<usize>,
    request_page: Option<usize>,
    in_flight: Option<Receiver<Result<PdfPreviewPage, String>>>,
    failed: bool,
}

impl PdfPanelState {
    fn reset_for(&mut self, file: &FileEntry) {
        *self = Self {
            path: Some(file.path.clone()),
            modified: file.modified,
            size: file.size,
            ..Default::default()
        };
    }

    fn start_request(&mut self) {
        let Some(path) = self.path.clone() else {
            return;
        };
        match request_page(path, self.page) {
            Ok(receiver) => {
                self.request_page = Some(self.page);
                self.in_flight = Some(receiver);
                self.failed = false;
            }
            Err(_) => self.failed = true,
        }
    }
}

pub(super) fn render_pdf_preview(
    ui: &mut egui::Ui,
    file: &FileEntry,
    svg_manager: &mut SvgIconManager,
) {
    let id = egui::Id::new("detail_pdf_preview_state");
    let mut state: PdfPanelState = ui.ctx().data_mut(|data| {
        let mut state: PdfPanelState = data.get_temp(id).unwrap_or_default();
        if state.path.as_ref() != Some(&file.path)
            || state.modified != file.modified
            || state.size != file.size
        {
            state.reset_for(file);
        }
        state
    });

    if let Some(receiver) = state.in_flight.as_ref() {
        match receiver.try_recv() {
            Ok(Ok(result)) => {
                state.page_count = result.page_count;
                if state.request_page == Some(result.page_index) && state.page == result.page_index
                {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [result.width as usize, result.height as usize],
                        &result.pixels,
                    );
                    state.texture = Some(ui.ctx().load_texture(
                        "detail_pdf_preview_page",
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                    state.texture_page = Some(result.page_index);
                }
                state.in_flight = None;
                state.request_page = None;
                state.failed = false;
            }
            Ok(Err(_)) => {
                if state.request_page == Some(state.page) {
                    state.failed = true;
                }
                state.in_flight = None;
                state.request_page = None;
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                state.in_flight = None;
                state.request_page = None;
                state.failed = true;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }

    super::image_preview::render_centered_preview(ui, |ui| {
        let step = show_centered_pagination(ui, &state).inner;
        if step != 0 {
            state.page = state.page.saturating_add_signed(step);
            state.failed = false;
        }

        if state.texture_page != Some(state.page) && state.in_flight.is_none() && !state.failed {
            state.start_request();
        }

        show_page_content(ui, file, &state, svg_manager);
    });

    ui.ctx().data_mut(|data| data.insert_temp(id, state));
}

fn show_page_content(
    ui: &mut egui::Ui,
    file: &FileEntry,
    state: &PdfPanelState,
    svg_manager: &mut SvgIconManager,
) {
    if state.failed {
        ui.colored_label(
            egui::Color32::from_rgb(232, 17, 35),
            rust_i18n::t!("preview.content_unavailable").to_string(),
        );
    } else if let Some(texture) = &state.texture {
        // Keep the last completed page visible until the requested page has
        // been rendered and its texture replaces this one.
        let _ = render_texture_with_overlay(ui, file, texture, svg_manager);
    } else {
        show_loading(ui);
    }
}

fn show_centered_pagination(
    ui: &mut egui::Ui,
    state: &PdfPanelState,
) -> egui::InnerResponse<isize> {
    let mut sizing_ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("pdf_pagination_measure")
            .sizing_pass()
            .invisible()
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    pagination_buttons(&mut sizing_ui, state);
    let size = sizing_ui.min_size();
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| pagination_buttons(ui, state),
    )
}

fn pagination_buttons(ui: &mut egui::Ui, state: &PdfPanelState) -> isize {
    let previous = ui
        .add_enabled(
            state.page > 0 && state.in_flight.is_none(),
            egui::Button::new("◀").small(),
        )
        .on_hover_text(rust_i18n::t!("preview.pdf_previous").to_string())
        .clicked();
    if state.page_count > 0 {
        ui.label(rust_i18n::t!(
            "preview.pdf_page",
            current = state.page + 1,
            total = state.page_count
        ));
    } else {
        ui.label("…");
    }
    let next = ui
        .add_enabled(
            state.page + 1 < state.page_count && state.in_flight.is_none(),
            egui::Button::new("▶").small(),
        )
        .on_hover_text(rust_i18n::t!("preview.pdf_next").to_string())
        .clicked();
    isize::from(next) - isize::from(previous)
}

fn show_loading(ui: &mut egui::Ui) {
    ui.add(egui::Spinner::new());
    ui.label(rust_i18n::t!("preview.content_loading").to_string());
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_previous_page_height_while_the_next_page_loads() {
        let context = egui::Context::default();
        let texture = context.load_texture(
            "pdf_preview_test_page",
            egui::ColorImage::filled([180, 240], egui::Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let file = FileEntry::from_path(PathBuf::from("sample.pdf"), false);
        let mut svg_manager = SvgIconManager::default();
        let mut measured_heights = Vec::new();
        let (_pending_reply, pending_result) = bounded::<Result<PdfPreviewPage, String>>(1);

        for state in [
            PdfPanelState {
                page: 0,
                texture: Some(texture.clone()),
                texture_page: Some(0),
                ..Default::default()
            },
            PdfPanelState {
                page: 1,
                texture: Some(texture.clone()),
                texture_page: Some(0),
                in_flight: Some(pending_result),
                ..Default::default()
            },
        ] {
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    super::super::image_preview::render_centered_preview(ui, |ui| {
                        show_page_content(ui, &file, &state, &mut svg_manager);
                        measured_heights.push(ui.min_rect().height());
                    });
                },
            );
        }

        assert_eq!(measured_heights.len(), 2);
        assert!(measured_heights[0] > 150.0);
        assert!(
            (measured_heights[0] - measured_heights[1]).abs() < 0.1,
            "preview height changed while waiting for the next page: {measured_heights:?}"
        );
    }

    #[test]
    fn pagination_is_centered_without_consuming_the_panel_height() {
        for width in [240.0, 600.0] {
            let context = egui::Context::default();
            let mut rect = egui::Rect::NOTHING;
            let mut center = 0.0;
            let _ = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    center = ui.available_rect_before_wrap().center().x;
                    super::super::image_preview::render_centered_preview(ui, |ui| {
                        rect = show_centered_pagination(
                            ui,
                            &PdfPanelState {
                                page_count: 20,
                                ..Default::default()
                            },
                        )
                        .response
                        .rect;
                    });
                },
            );
            assert!((rect.center().x - center).abs() < 1.0);
            assert!(rect.height() < 60.0);
        }
    }
}
