use eframe::egui;

use super::{DocxViewerApp, PageGeometry, ZoomMode};

const PAGE_GAP: f32 = 24.0;
const PAGE_CACHE_RADIUS: usize = 2;
const PAGE_CACHE_BUDGET: usize = 128 * 1024 * 1024;

impl DocxViewerApp {
    pub(super) fn show_pages(&mut self, ui: &mut egui::Ui) {
        let available_width = (ui.available_width() - 32.0).max(120.0);
        let available_height = (ui.available_height() - 24.0).max(120.0);
        let geometries = self.page_geometries(available_width, available_height);
        self.effective_zoom = self.current_scale(available_width, available_height);
        let page_width = geometries
            .iter()
            .map(|geometry| geometry.size.x)
            .fold(available_width, f32::max);
        let content_width = page_width + 32.0;
        let content_height = geometries.last().map_or(0.0, |last| last.rect.bottom()) + 16.0;
        let scroll_target = self
            .scroll_to_match
            .take()
            .and_then(|(page, fraction)| {
                geometries
                    .get(page)
                    .map(|geometry| geometry.rect.top() + geometry.size.y * fraction)
            })
            .or_else(|| {
                self.scroll_to_page
                    .take()
                    .and_then(|page| geometries.get(page).map(|geometry| geometry.rect.top()))
            });

        let mut scroll_area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if let Some(target) = scroll_target {
            scroll_area = scroll_area.vertical_scroll_offset(target);
        }
        let mut visible_pages = Vec::new();
        scroll_area.show_viewport(ui, |ui, viewport| {
            ui.set_min_size(egui::vec2(content_width, content_height));
            let origin = ui.max_rect().min.to_vec2();
            for (page_index, geometry) in geometries.iter().enumerate() {
                let x = (content_width - geometry.size.x) * 0.5;
                let relative_rect =
                    egui::Rect::from_min_size(egui::pos2(x, geometry.rect.top()), geometry.size);
                if !relative_rect.intersects(viewport) {
                    continue;
                }
                visible_pages.push(page_index);
                let rect = relative_rect.translate(origin);

                ui.painter().rect_filled(
                    rect.translate(egui::vec2(0.0, 2.0)),
                    2.0,
                    egui::Color32::from_black_alpha(36),
                );
                ui.painter().rect_filled(rect, 1.0, egui::Color32::WHITE);

                if let Some(page) = self.textures.get(&page_index) {
                    ui.painter().image(
                        page.texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                } else if let Some(error) = self.failed_pages.get(&page_index) {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        error,
                        egui::FontId::proportional(14.0),
                        egui::Color32::RED,
                    );
                } else {
                    self.start_page_request(page_index);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "…",
                        egui::FontId::proportional(24.0),
                        egui::Color32::GRAY,
                    );
                }
                let page_scale = self
                    .page_sizes
                    .get(page_index)
                    .map_or(1.0, |(width, _)| geometry.size.x / *width);
                self.paint_search_highlights(ui.painter(), page_index, rect, page_scale);
                let response = ui.interact(
                    rect,
                    ui.id().with(("docx_page_selection", page_index)),
                    egui::Sense::click_and_drag(),
                );
                self.handle_page_selection(ui, &response, page_index, rect, page_scale);
            }
            if let Some((page_index, _)) = geometries
                .iter()
                .enumerate()
                .find(|(_, geometry)| geometry.rect.center().y >= viewport.center().y)
            {
                self.current_page = page_index;
            }
        });

        self.evict_distant_pages(&visible_pages);
    }

    fn page_geometries(&self, available_width: f32, available_height: f32) -> Vec<PageGeometry> {
        let mut top = 8.0;
        self.page_sizes
            .iter()
            .map(|(width, height)| {
                let scale = match self.zoom_mode {
                    ZoomMode::FitWidth => (available_width / width).clamp(0.25, 4.0),
                    ZoomMode::FitPage => (available_width / width)
                        .min(available_height / height)
                        .clamp(0.25, 4.0),
                    ZoomMode::Custom => self.zoom,
                };
                let size = egui::vec2(width * scale, height * scale);
                let rect = egui::Rect::from_min_size(egui::pos2(0.0, top), size);
                top += size.y + PAGE_GAP;
                PageGeometry { rect, size }
            })
            .collect()
    }

    fn evict_distant_pages(&mut self, visible_pages: &[usize]) {
        let center = self.current_page;
        let mut evict = self
            .textures
            .keys()
            .copied()
            .filter(|page| {
                !visible_pages.contains(page) && page.abs_diff(center) > PAGE_CACHE_RADIUS
            })
            .collect::<Vec<_>>();
        evict.sort_by_key(|page| page.abs_diff(center));
        for page in evict {
            if let Some(texture) = self.textures.remove(&page) {
                self.cache_bytes = self.cache_bytes.saturating_sub(texture.bytes);
            }
        }

        if self.cache_bytes > PAGE_CACHE_BUDGET {
            let mut distant = self
                .textures
                .keys()
                .copied()
                .filter(|page| !visible_pages.contains(page))
                .collect::<Vec<_>>();
            distant.sort_by_key(|page| std::cmp::Reverse(page.abs_diff(center)));
            for page in distant {
                if self.cache_bytes <= PAGE_CACHE_BUDGET {
                    break;
                }
                if let Some(texture) = self.textures.remove(&page) {
                    self.cache_bytes = self.cache_bytes.saturating_sub(texture.bytes);
                }
            }
        }
    }
}
