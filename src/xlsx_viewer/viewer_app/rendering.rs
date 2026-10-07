use super::{RenderKey, RenderedTextureTile, XlsxSearchMatch, XlsxViewerApp};
use betteroffice_xlsx::{Viewport, MAX_PIXMAP_DIM};
use eframe::egui;

impl XlsxViewerApp {
    pub(super) fn show_sheet(&mut self, ui: &mut egui::Ui) {
        let Some(sheet_info) = self.sheet_info.clone() else {
            ui.centered_and_justified(|ui| ui.spinner());
            return;
        };

        let pixels_per_point = ui.ctx().pixels_per_point().max(0.5);
        let content_size = egui::vec2(
            content_size_points(
                sheet_info.content_width,
                self.zoom,
                pixels_per_point,
                ui.available_width(),
            ),
            content_size_points(
                sheet_info.content_height,
                self.zoom,
                pixels_per_point,
                ui.available_height(),
            ),
        );
        let scroll_target = self.scroll_to_position.take();
        let mut scroll_area = egui::ScrollArea::both()
            .id_salt(("xlsx_sheet_content", self.selected_sheet))
            .auto_shrink([false, false]);
        if let Some((x, y)) = scroll_target {
            scroll_area = scroll_area
                .horizontal_scroll_offset((x * self.zoom / pixels_per_point).max(0.0))
                .vertical_scroll_offset((y * self.zoom / pixels_per_point).max(0.0));
        }

        let mut render_key = None;
        scroll_area.show_viewport(ui, |ui, viewport| {
            ui.set_min_size(content_size);
            let key = RenderKey {
                sheet_index: self.selected_sheet,
                viewport: make_render_viewport(
                    viewport,
                    pixels_per_point,
                    self.zoom,
                    sheet_info.content_width,
                    sheet_info.content_height,
                ),
                zoom: self.zoom,
            };
            render_key = Some(key);

            let current_render_failed = self
                .render_error
                .as_ref()
                .is_some_and(|(error_key, _)| *error_key == key);
            if let Some(texture) = self.texture.as_ref().filter(|texture| {
                !current_render_failed
                    && texture.key.sheet_index == key.sheet_index
                    && texture.key.zoom == key.zoom
            }) {
                let stale_render = texture.key != key;
                let viewport_delta = rendered_viewport_delta(texture.key, key, pixels_per_point);
                let painter = ui.painter();
                let clip_origin = ui.clip_rect().min;
                let scale = self.zoom / pixels_per_point;
                for tile in &texture.tiles {
                    let body_width = (tile.viewport.width - tile.frozen_width).max(0.0);
                    let body_height = (tile.viewport.height - tile.frozen_height).max(0.0);
                    if tile.body_offset_x == 0.0 && tile.body_offset_y == 0.0 {
                        paint_tile_region(
                            painter,
                            clip_origin,
                            tile,
                            egui::Rect::from_min_size(
                                egui::pos2(0.0, 0.0),
                                egui::vec2(tile.frozen_width, tile.frozen_height),
                            ),
                            egui::vec2(0.0, 0.0),
                            scale,
                        );
                    }
                    if tile.body_offset_y == 0.0 {
                        paint_tile_region(
                            painter,
                            clip_origin,
                            tile,
                            egui::Rect::from_min_size(
                                egui::pos2(tile.frozen_width, 0.0),
                                egui::vec2(body_width, tile.frozen_height),
                            ),
                            egui::vec2(
                                tile.frozen_width + tile.body_offset_x + viewport_delta.x,
                                0.0,
                            ),
                            scale,
                        );
                    }
                    if tile.body_offset_x == 0.0 {
                        paint_tile_region(
                            painter,
                            clip_origin,
                            tile,
                            egui::Rect::from_min_size(
                                egui::pos2(0.0, tile.frozen_height),
                                egui::vec2(tile.frozen_width, body_height),
                            ),
                            egui::vec2(
                                0.0,
                                tile.frozen_height + tile.body_offset_y + viewport_delta.y,
                            ),
                            scale,
                        );
                    }
                    paint_tile_region(
                        painter,
                        clip_origin,
                        tile,
                        egui::Rect::from_min_size(
                            egui::pos2(tile.frozen_width, tile.frozen_height),
                            egui::vec2(body_width, body_height),
                        ),
                        egui::vec2(
                            tile.frozen_width + tile.body_offset_x + viewport_delta.x,
                            tile.frozen_height + tile.body_offset_y + viewport_delta.y,
                        ),
                        scale,
                    );
                }
                self.paint_search_highlights(ui, key, pixels_per_point);
                if stale_render {
                    self.request_render(key);
                }
            } else if let Some((_, error)) = self
                .render_error
                .as_ref()
                .filter(|(error_key, _)| *error_key == key)
            {
                ui.painter().text(
                    ui.clip_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    error,
                    egui::FontId::proportional(14.0),
                    egui::Color32::RED,
                );
            } else {
                self.request_render(key);
                ui.painter().text(
                    ui.clip_rect().center(),
                    egui::Align2::CENTER_CENTER,
                    "…",
                    egui::FontId::proportional(24.0),
                    egui::Color32::GRAY,
                );
            }
        });

        self.latest_render_key = render_key;
    }

    fn paint_search_highlights(&self, ui: &egui::Ui, key: RenderKey, pixels_per_point: f32) {
        let painter = ui.painter();
        let screen_origin = ui.clip_rect().min.to_vec2();
        for (index, search_match) in search_matches_for_sheet(&self.search_results, key.sheet_index)
        {
            let Some(rect) = search_match_rect(search_match, key, pixels_per_point) else {
                continue;
            };
            let rect = rect.translate(screen_origin);
            let (fill, stroke) = search_highlight_style(index == self.current_match_idx);
            painter.rect_filled(rect, 1.5, fill);
            painter.rect(
                rect,
                1.5,
                egui::Color32::TRANSPARENT,
                stroke,
                egui::StrokeKind::Outside,
            );
        }
    }

    fn request_render(&mut self, key: RenderKey) {
        if self.pending_render.is_some() {
            return;
        }
        let generation = self.next_render_generation;
        let requested = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.request_render(generation, key));
        if requested {
            self.next_render_generation = self.next_render_generation.wrapping_add(1);
            self.pending_render = Some(generation);
            self.render_error = None;
        }
    }
}

fn rendered_viewport_delta(
    rendered_key: RenderKey,
    current_key: RenderKey,
    pixels_per_point: f32,
) -> egui::Vec2 {
    let scale = current_key.zoom / pixels_per_point;
    egui::vec2(
        (rendered_key.viewport.x - current_key.viewport.x) * scale,
        (rendered_key.viewport.y - current_key.viewport.y) * scale,
    )
}

fn search_matches_for_sheet<'a>(
    matches: &'a [XlsxSearchMatch],
    sheet_index: usize,
) -> impl Iterator<Item = (usize, &'a XlsxSearchMatch)> + 'a {
    matches
        .iter()
        .enumerate()
        .filter(move |(_, search_match)| search_match.sheet_index == sheet_index)
}

fn search_highlight_style(is_current: bool) -> (egui::Color32, egui::Stroke) {
    if is_current {
        (
            egui::Color32::from_rgba_unmultiplied(255, 190, 40, 48),
            egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(230, 105, 0, 230)),
        )
    } else {
        (
            egui::Color32::from_rgba_unmultiplied(70, 170, 255, 34),
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(20, 120, 210, 190),
            ),
        )
    }
}

fn search_match_rect(
    search_match: &XlsxSearchMatch,
    key: RenderKey,
    pixels_per_point: f32,
) -> Option<egui::Rect> {
    let bounds = search_match.bounds;
    let scale = key.zoom / pixels_per_point;
    if !scale.is_finite()
        || scale <= 0.0
        || !bounds.left.is_finite()
        || !bounds.top.is_finite()
        || !bounds.right.is_finite()
        || !bounds.bottom.is_finite()
    {
        return None;
    }

    let frozen_width = search_match.frozen_width.min(key.viewport.width).max(0.0);
    let frozen_height = search_match.frozen_height.min(key.viewport.height).max(0.0);
    let frozen_col = search_match.cell.col < search_match.frozen_cols;
    let frozen_row = search_match.cell.row < search_match.frozen_rows;
    let left = if frozen_col {
        bounds.left
    } else {
        bounds.left - key.viewport.x
    };
    let right = if frozen_col {
        bounds.right
    } else {
        bounds.right - key.viewport.x
    };
    let top = if frozen_row {
        bounds.top
    } else {
        bounds.top - key.viewport.y
    };
    let bottom = if frozen_row {
        bounds.bottom
    } else {
        bounds.bottom - key.viewport.y
    };
    let pane_clip = egui::Rect::from_min_max(
        egui::pos2(
            if frozen_col { 0.0 } else { frozen_width },
            if frozen_row { 0.0 } else { frozen_height },
        ),
        egui::pos2(
            if frozen_col {
                frozen_width
            } else {
                key.viewport.width
            },
            if frozen_row {
                frozen_height
            } else {
                key.viewport.height
            },
        ),
    );
    let viewport_clip = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(key.viewport.width, key.viewport.height),
    );
    let rect = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom))
        .intersect(pane_clip)
        .intersect(viewport_clip);
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    Some(egui::Rect::from_min_max(
        egui::pos2(rect.left() * scale, rect.top() * scale),
        egui::pos2(rect.right() * scale, rect.bottom() * scale),
    ))
}

fn paint_tile_region(
    painter: &egui::Painter,
    clip_origin: egui::Pos2,
    tile: &RenderedTextureTile,
    source: egui::Rect,
    destination_origin: egui::Vec2,
    scale: f32,
) {
    if source.width() <= 0.0 || source.height() <= 0.0 {
        return;
    }
    let uv = egui::Rect::from_min_max(
        egui::pos2(
            source.min.x / tile.viewport.width,
            source.min.y / tile.viewport.height,
        ),
        egui::pos2(
            source.max.x / tile.viewport.width,
            source.max.y / tile.viewport.height,
        ),
    );
    let rect = egui::Rect::from_min_size(
        clip_origin + destination_origin * scale,
        source.size() * scale,
    );
    painter.image(tile.texture.id(), rect, uv, egui::Color32::WHITE);
}

fn content_size_points(extent: f32, zoom: f32, pixels_per_point: f32, available: f32) -> f32 {
    let size = f64::from(extent) * f64::from(zoom) / f64::from(pixels_per_point);
    if size.is_finite() && size > 0.0 {
        size.max(f64::from(available))
            .min(f64::from(f32::MAX / 4.0)) as f32
    } else {
        available.max(1.0)
    }
}

fn make_render_viewport(
    viewport: egui::Rect,
    pixels_per_point: f32,
    zoom: f32,
    content_width: f32,
    content_height: f32,
) -> Viewport {
    let scale = f64::from(pixels_per_point) / f64::from(zoom);
    let (x, width) = bounded_axis(
        f64::from(viewport.min.x) * scale,
        f64::from(viewport.width()) * scale,
        content_width,
    );
    let (y, height) = bounded_axis(
        f64::from(viewport.min.y) * scale,
        f64::from(viewport.height()) * scale,
        content_height,
    );
    Viewport {
        x,
        y,
        width,
        height,
    }
}

fn bounded_axis(offset: f64, size: f64, content_extent: f32) -> (f32, f32) {
    let max_dimension = f64::from(MAX_PIXMAP_DIM);
    let extent = if content_extent.is_finite() && content_extent > 0.0 {
        f64::from(content_extent)
    } else {
        max_dimension
    };
    let size_limit = extent.min(max_dimension).max(1.0);
    let size = if size.is_finite() && size > 0.0 {
        size.min(size_limit)
    } else {
        size_limit
    };
    let max_offset = (extent - size - 1.0).max(0.0);
    let offset = if offset.is_nan() {
        0.0
    } else {
        offset.max(0.0).min(max_offset)
    };
    (offset as f32, size.max(1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::super::super::renderer::XlsxCellBounds;
    use super::{
        make_render_viewport, rendered_viewport_delta, search_highlight_style, search_match_rect,
        search_matches_for_sheet, RenderKey, XlsxSearchMatch,
    };
    use betteroffice_xlsx::{CellRef, Viewport};
    use eframe::egui;

    fn make_search_match(cell: CellRef, bounds: XlsxCellBounds) -> XlsxSearchMatch {
        XlsxSearchMatch {
            sheet_index: 0,
            cell,
            address: "A1".to_owned(),
            text: "match".to_owned(),
            bounds,
            frozen_rows: 2,
            frozen_cols: 1,
            frozen_width: 60.0,
            frozen_height: 40.0,
            scroll_position: (0.0, 0.0),
        }
    }

    #[test]
    fn cached_viewport_moves_opposite_to_scroll_delta() {
        let rendered = RenderKey {
            sheet_index: 0,
            viewport: Viewport {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            },
            zoom: 1.5,
        };
        let current = RenderKey {
            viewport: Viewport {
                x: 140.0,
                y: 80.0,
                ..rendered.viewport
            },
            ..rendered
        };
        assert_eq!(
            rendered_viewport_delta(rendered, current, 1.5),
            egui::vec2(-40.0, -30.0),
        );
    }

    #[test]
    fn search_highlights_follow_scroll_zoom_and_frozen_panes() {
        let key = RenderKey {
            sheet_index: 0,
            viewport: Viewport {
                x: 100.0,
                y: 50.0,
                width: 400.0,
                height: 300.0,
            },
            zoom: 1.5,
        };
        let body = make_search_match(
            CellRef::new(10, 5),
            XlsxCellBounds {
                left: 240.0,
                top: 150.0,
                right: 320.0,
                bottom: 180.0,
            },
        );
        assert_eq!(
            search_match_rect(&body, key, 2.0),
            Some(egui::Rect::from_min_max(
                egui::pos2(105.0, 75.0),
                egui::pos2(165.0, 97.5),
            )),
        );

        let frozen = make_search_match(
            CellRef::new(0, 0),
            XlsxCellBounds {
                left: 0.0,
                top: 0.0,
                right: 60.0,
                bottom: 20.0,
            },
        );
        assert_eq!(
            search_match_rect(&frozen, key, 2.0),
            Some(egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(45.0, 15.0),
            )),
        );

        let clipped = make_search_match(
            CellRef::new(10, 5),
            XlsxCellBounds {
                left: 140.0,
                top: 100.0,
                right: 180.0,
                bottom: 130.0,
            },
        );
        assert_eq!(
            search_match_rect(&clipped, key, 2.0),
            Some(egui::Rect::from_min_max(
                egui::pos2(45.0, 37.5),
                egui::pos2(60.0, 60.0),
            )),
        );
    }

    #[test]
    fn search_highlights_only_the_selected_sheet_and_emphasizes_current_match() {
        let first = make_search_match(
            CellRef::new(1, 2),
            XlsxCellBounds {
                left: 10.0,
                top: 10.0,
                right: 30.0,
                bottom: 30.0,
            },
        );
        let mut other_sheet = make_search_match(
            CellRef::new(3, 4),
            XlsxCellBounds {
                left: 40.0,
                top: 40.0,
                right: 60.0,
                bottom: 60.0,
            },
        );
        other_sheet.sheet_index = 1;
        let matches = [first, other_sheet];
        let visible = search_matches_for_sheet(&matches, 0)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(visible, vec![0]);

        let current = search_highlight_style(true);
        let other = search_highlight_style(false);
        assert_ne!(current.0, other.0);
        assert_ne!(current.1.color, other.1.color);
    }

    #[test]
    fn viewport_stays_inside_sheet_extents_at_100_and_144_percent_zoom() {
        let ui_viewport = egui::Rect::from_min_size(egui::pos2(99.5, 49.5), egui::vec2(20.0, 20.0));

        for zoom in [1.0, 1.44] {
            let viewport = make_render_viewport(ui_viewport, 1.0, zoom, 100.0, 50.0);
            assert!(viewport.x.is_finite());
            assert!(viewport.y.is_finite());
            assert!(viewport.width.is_finite() && viewport.width > 0.0);
            assert!(viewport.height.is_finite() && viewport.height > 0.0);
            assert!(viewport.x + viewport.width <= 100.0);
            assert!(viewport.y + viewport.height <= 50.0);
        }
    }

    #[test]
    fn viewport_clamps_oversized_and_non_finite_requests() {
        let viewports = [
            egui::Rect::from_min_size(
                egui::pos2(f32::MAX / 2.0, f32::MAX / 2.0),
                egui::vec2(f32::MAX / 2.0, f32::MAX / 2.0),
            ),
            egui::Rect::from_min_max(
                egui::pos2(f32::INFINITY, f32::INFINITY),
                egui::pos2(f32::INFINITY, f32::INFINITY),
            ),
        ];

        for ui_viewport in viewports {
            let viewport = make_render_viewport(ui_viewport, 2.0, 1.0, 20_000.0, 20_000.0);
            assert!(viewport.x.is_finite());
            assert!(viewport.y.is_finite());
            assert!(viewport.width.is_finite() && viewport.width > 0.0);
            assert!(viewport.height.is_finite() && viewport.height > 0.0);
            assert!(viewport.width <= betteroffice_xlsx::MAX_PIXMAP_DIM as f32);
            assert!(viewport.height <= betteroffice_xlsx::MAX_PIXMAP_DIM as f32);
            assert!(viewport.x + viewport.width <= 20_000.0);
            assert!(viewport.y + viewport.height <= 20_000.0);
        }
    }
}
