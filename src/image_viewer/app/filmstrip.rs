use crate::image_viewer::loader;
use crate::ui::theme;
use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;
use eframe::egui::{Rect, Sense};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

// Filmstrip constants
pub(super) const FILMSTRIP_THUMB_SIZE: f32 = 80.0;
const FILMSTRIP_SPACING: f32 = 4.0;
const FILMSTRIP_PANEL_HEIGHT: f32 = 88.0;
const FILMSTRIP_OVERSCAN: usize = 20;
const FILMSTRIP_MAX_CACHED: usize = 96;
pub(super) const FILMSTRIP_DECODE_MAX_SIDE: u32 = 160;
const FILMSTRIP_MAX_UPLOADS_PER_FRAME: usize = 8;
const FILMSTRIP_MAX_IN_FLIGHT: usize = 16;

pub(in crate::image_viewer) struct FilmstripState {
    pub(super) thumbnails: HashMap<usize, egui::TextureHandle>,
    pub(super) pending: HashSet<usize>,
    pub(super) cache_misses: HashSet<usize>,
    pub(super) result_tx: crossbeam_channel::Sender<(usize, u64, loader::DecodedFrame)>,
    pub(super) result_rx: crossbeam_channel::Receiver<(usize, u64, loader::DecodedFrame)>,
    pub(super) generation: u64,
    pub(super) scroll_to_current: bool,
    pub(super) last_viewport_width: Option<f32>,
}

impl FilmstripState {
    pub(super) fn new() -> Self {
        let (result_tx, result_rx) = crossbeam_channel::bounded(64);
        Self {
            thumbnails: HashMap::new(),
            pending: HashSet::new(),
            cache_misses: HashSet::new(),
            result_tx,
            result_rx,
            generation: 0,
            scroll_to_current: true,
            last_viewport_width: None,
        }
    }

    pub(super) fn reset(&mut self) {
        let (result_tx, result_rx) = crossbeam_channel::bounded(64);
        self.result_tx = result_tx;
        self.result_rx = result_rx;
        self.thumbnails.clear();
        self.pending.clear();
        self.cache_misses.clear();
        self.generation = self.generation.wrapping_add(1);
        self.scroll_to_current = true;
        self.last_viewport_width = None;
    }
}

impl super::DedicatedImageViewerApp {
    pub(super) fn poll_filmstrip_results(&mut self, ctx: &egui::Context) {
        let mut uploads = 0;
        while uploads < FILMSTRIP_MAX_UPLOADS_PER_FRAME {
            let Ok((index, gen, frame)) = self.filmstrip.result_rx.try_recv() else {
                break;
            };
            if gen != self.filmstrip.generation {
                continue;
            }
            self.filmstrip.pending.remove(&index);
            if frame.width == 0 || frame.height == 0 || frame.rgba.is_empty() {
                self.filmstrip.cache_misses.insert(index);
                continue;
            }
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width as usize, frame.height as usize],
                &frame.rgba,
            );
            let texture = ctx.load_texture(
                format!("filmstrip_{}_{}", self.filmstrip.generation, index),
                color_image,
                egui::TextureOptions::LINEAR,
            );
            self.filmstrip.thumbnails.insert(index, texture);
            uploads += 1;
        }
    }

    /// Proactively decode filmstrip thumbnails around the current image so
    /// they appear immediately when the user scrolls or clicks neighbours.
    pub(super) fn prefetch_filmstrip_neighbors(&mut self) {
        let total = self.sequence.entries.len();
        if total == 0 {
            return;
        }

        let center = self.current_index;
        let half_visible = 6usize;
        let start = center.saturating_sub(half_visible);
        let end = (center + half_visible).min(total - 1);

        self.request_filmstrip_thumbnails(start..=end);
    }

    pub(super) fn evict_filmstrip_textures(&mut self) {
        if self.filmstrip.thumbnails.len() <= FILMSTRIP_MAX_CACHED {
            return;
        }
        let center = self.current_index;
        let mut indices: Vec<usize> = self.filmstrip.thumbnails.keys().copied().collect();
        indices.sort_by_key(|i| std::cmp::Reverse(i.abs_diff(center)));
        let to_remove = self.filmstrip.thumbnails.len() - FILMSTRIP_MAX_CACHED;
        for idx in indices.into_iter().take(to_remove) {
            self.filmstrip.thumbnails.remove(&idx);
        }
    }

    pub(super) fn render_filmstrip(&mut self, root_ui: &mut egui::Ui) {
        if self.sequence.entries.len() <= 1 {
            return;
        }

        let ctx = root_ui.ctx().clone();
        let total = self.sequence.entries.len();
        let current = self.current_index;
        let item_w = FILMSTRIP_THUMB_SIZE + FILMSTRIP_SPACING;
        let total_content_w = total as f32 * item_w + FILMSTRIP_SPACING;

        egui::Panel::bottom("filmstrip_panel")
            .exact_size(FILMSTRIP_PANEL_HEIGHT)
            .show(root_ui, |ui| {
                let panel_bg = if ui.visuals().dark_mode {
                    egui::Color32::from_gray(30)
                } else {
                    egui::Color32::from_rgb(238, 238, 238)
                };
                ui.painter().rect_filled(ui.max_rect(), 0.0, panel_bg);
                let sep_color = if ui.visuals().dark_mode {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(225)
                };
                ui.painter().hline(
                    ui.max_rect().x_range(),
                    ui.max_rect().min.y,
                    egui::Stroke::new(1.0, sep_color),
                );

                let viewport_width = ui.available_width().max(0.0);
                let viewport_resized = self
                    .filmstrip
                    .last_viewport_width
                    .is_none_or(|previous| (previous - viewport_width).abs() > 0.5);
                self.filmstrip.last_viewport_width = Some(viewport_width);

                let should_scroll = self.filmstrip.scroll_to_current || viewport_resized;
                self.filmstrip.scroll_to_current = false;

                let mut scroll_area = egui::ScrollArea::horizontal()
                    .id_salt("filmstrip_scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden);
                if should_scroll {
                    scroll_area = scroll_area.horizontal_scroll_offset(centered_scroll_offset(
                        current,
                        total_content_w,
                        viewport_width,
                    ));
                }

                let scroll_output = scroll_area.show_viewport(ui, |ui, viewport| {
                    ui.spacing_mut().item_spacing = egui::vec2(FILMSTRIP_SPACING, 0.0);
                    ui.set_min_width(total_content_w);

                    let first_visible = (viewport.min.x / item_w).floor().max(0.0) as usize;
                    let last_visible = (viewport.max.x / item_w).ceil().max(0.0) as usize;
                    let start = first_visible.saturating_sub(FILMSTRIP_OVERSCAN);
                    let end = (last_visible + FILMSTRIP_OVERSCAN + 1).min(total);

                    let content_left = ui.max_rect().left();
                    let top = ui.max_rect().top();

                    self.request_filmstrip_thumbnails(start..end);

                    for idx in start..end {
                        let rect = Rect::from_min_size(
                            egui::pos2(content_left + idx as f32 * item_w, top),
                            egui::vec2(FILMSTRIP_THUMB_SIZE, FILMSTRIP_THUMB_SIZE),
                        );

                        let response = ui.interact(
                            rect,
                            ui.id().with(("filmstrip_item", idx)),
                            Sense::click(),
                        );
                        self.paint_filmstrip_item(ui, idx, rect, response.hovered());

                        if response.clicked() {
                            self.navigate_to(idx, &ctx);
                        }
                    }
                });

                // Request repaint if we have pending thumbnails
                if !self.filmstrip.pending.is_empty() {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }

                let _ = scroll_output;
            });
    }

    fn request_filmstrip_thumbnails<I>(&mut self, indices: I)
    where
        I: IntoIterator<Item = usize>,
    {
        let available_slots = FILMSTRIP_MAX_IN_FLIGHT.saturating_sub(self.filmstrip.pending.len());
        if available_slots == 0 {
            return;
        }

        let mut requests: Vec<(usize, PathBuf)> = Vec::with_capacity(available_slots);
        for idx in indices {
            if requests.len() >= available_slots {
                break;
            }
            if self.filmstrip.thumbnails.contains_key(&idx)
                || self.filmstrip.pending.contains(&idx)
                || self.filmstrip.cache_misses.contains(&idx)
            {
                continue;
            }
            if let Some(path) = self.sequence.entries.get(idx).cloned() {
                self.filmstrip.pending.insert(idx);
                requests.push((idx, path));
            }
        }

        if requests.is_empty() {
            return;
        }

        let tx = self.filmstrip.result_tx.clone();
        let gen = self.filmstrip.generation;
        rayon::spawn(move || {
            let paths: Vec<PathBuf> = requests.iter().map(|(_, path)| path.clone()).collect();
            let frames =
                loader::try_fast_previews_from_disk_cache(&paths, FILMSTRIP_DECODE_MAX_SIDE);

            for ((idx, path), frame) in requests.into_iter().zip(frames) {
                let frame = load_filmstrip_frame(&path, frame);
                let _ = tx.try_send((idx, gen, frame));
            }
        });
    }

    fn paint_filmstrip_item(&self, ui: &mut egui::Ui, idx: usize, rect: Rect, hovered: bool) {
        let is_current = idx == self.current_index;
        let bg_color = if is_current {
            if ui.visuals().dark_mode {
                egui::Color32::from_gray(50)
            } else {
                egui::Color32::WHITE
            }
        } else if hovered {
            if ui.visuals().dark_mode {
                egui::Color32::from_gray(45)
            } else {
                egui::Color32::from_gray(228)
            }
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 4.0, bg_color);

        if let Some(tex) = self.filmstrip.thumbnails.get(&idx) {
            let tex_size = tex.size_vec2();
            let scale = if tex_size.x > 0.0 && tex_size.y > 0.0 {
                let sx = (FILMSTRIP_THUMB_SIZE - 4.0) / tex_size.x;
                let sy = (FILMSTRIP_THUMB_SIZE - 4.0) / tex_size.y;
                sx.min(sy)
            } else {
                1.0
            };
            let draw_size = tex_size * scale;
            let image_rect = Rect::from_center_size(rect.center(), draw_size);
            ui.painter().image(
                tex.id(),
                image_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        if is_current {
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(2.0, theme::COLOR_ACCENT),
                egui::StrokeKind::Outside,
            );
        }
    }
}

fn centered_scroll_offset(current: usize, content_width: f32, viewport_width: f32) -> f32 {
    let item_width = FILMSTRIP_THUMB_SIZE + FILMSTRIP_SPACING;
    let current_center = current as f32 * item_width + FILMSTRIP_THUMB_SIZE * 0.5;
    let max_offset = (content_width - viewport_width).max(0.0);
    (current_center - viewport_width * 0.5).clamp(0.0, max_offset)
}

pub(super) fn empty_decoded_frame() -> loader::DecodedFrame {
    loader::DecodedFrame {
        rgba: Vec::new(),
        width: 0,
        height: 0,
        original_width: 0,
        original_height: 0,
    }
}

fn load_filmstrip_frame(
    path: &std::path::Path,
    cached: Option<loader::DecodedFrame>,
) -> loader::DecodedFrame {
    cached
        .or_else(|| {
            loader::decode_preview_frame_with_priority(
                path,
                FILMSTRIP_DECODE_MAX_SIDE,
                loader::DecodePriority::Background,
            )
            .ok()
        })
        .unwrap_or_else(empty_decoded_frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_offset_tracks_viewport_width() {
        let content_width = 40.0 * (FILMSTRIP_THUMB_SIZE + FILMSTRIP_SPACING) + FILMSTRIP_SPACING;

        let windowed = centered_scroll_offset(20, content_width, 800.0);
        let maximized = centered_scroll_offset(20, content_width, 1400.0);

        assert!(maximized < windowed);
        assert_eq!(windowed, 1320.0);
        assert_eq!(maximized, 1020.0);
    }

    #[test]
    fn centered_offset_clamps_at_content_edges() {
        let content_width = 20.0 * (FILMSTRIP_THUMB_SIZE + FILMSTRIP_SPACING) + FILMSTRIP_SPACING;

        assert_eq!(centered_scroll_offset(0, content_width, 800.0), 0.0);
        assert_eq!(
            centered_scroll_offset(19, content_width, 800.0),
            content_width - 800.0
        );
    }

    #[test]
    fn cache_miss_falls_back_to_source_preview() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();

        let frame = load_filmstrip_frame(&path, None);

        assert!(frame.width > 0);
        assert!(frame.height > 0);
        assert!(!frame.rgba.is_empty());
    }
}
