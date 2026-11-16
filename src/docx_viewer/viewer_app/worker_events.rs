use eframe::egui;

use super::{DocxRenderWorker, DocxViewerApp, PageTexture, ViewerStatus};
use crate::docx_viewer::render_worker::WorkerEvent;

impl DocxViewerApp {
    pub(super) fn poll_worker(&mut self, ctx: &egui::Context) {
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
                    let stages = self.opening_trace.texture_stages();
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
                    drop(stages);
                    self.opening_trace.texture_ready();
                }
                WorkerEvent::TextSelected {
                    page_index,
                    generation,
                    text,
                } => self.receive_selected_text(page_index, generation, text),
                WorkerEvent::PageFailed { page_index, error } => {
                    self.pending.remove(&page_index);
                    self.failed_pages.insert(page_index, error);
                }
                WorkerEvent::Failed(error) => {
                    self.status = ViewerStatus::Failed(error);
                }
            }
        }

        let search_results = self
            .worker
            .as_ref()
            .map(DocxRenderWorker::drain_search_results)
            .unwrap_or_default();
        for result in search_results {
            self.accept_search_result(result);
        }
    }
}
