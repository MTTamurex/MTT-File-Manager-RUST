use crate::image_viewer::loader;
use crate::ui::theme;
use eframe::egui;
use eframe::egui::scroll_area::ScrollBarVisibility;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

// Filmstrip constants
pub(super) const FILMSTRIP_THUMB_SIZE: f32 = 80.0;
const FILMSTRIP_SPACING: f32 = 4.0;
const FILMSTRIP_PANEL_HEIGHT: f32 = 88.0;
const FILMSTRIP_OVERSCAN: usize = 20;
const FILMSTRIP_MAX_CACHED: usize = 96;
pub(super) const FILMSTRIP_DECODE_MAX_SIDE: u32 = 160;
const FILMSTRIP_MAX_UPLOADS_PER_FRAME: usize = 6;

pub(in crate::image_viewer) struct FilmstripState {
    pub(super) thumbnails: HashMap<usize, egui::TextureHandle>,
    pub(super) pending: HashSet<usize>,
    pub(super) result_tx: crossbeam_channel::Sender<(usize, u64, loader::DecodedFrame)>,
    pub(super) result_rx: crossbeam_channel::Receiver<(usize, u64, loader::DecodedFrame)>,
    pub(super) generation: u64,
    pub(super) scroll_to_current: bool,
}

impl FilmstripState {
    pub(super) fn new() -> Self {
        let (result_tx, result_rx) = crossbeam_channel::bounded(64);
        Self {
            thumbnails: HashMap::new(),
            pending: HashSet::new(),
            result_tx,
            result_rx,
            generation: 0,
            scroll_to_current: true,
        }
    }

    pub(super) fn reset(&mut self) {
        self.thumbnails.clear();
        self.pending.clear();
        self.generation = self.generation.wrapping_add(1);
        self.scroll_to_current = true;
        // Drain any stale results from the old generation
        while self.result_rx.try_recv().is_ok() {}
    }
}

impl super::DedicatedImageViewerApp {
    pub(super) fn poll_filmstrip_results(&mut self, ctx: &egui::Context) {
        let mut uploads = 0;
        while let Ok((index, gen, frame)) = self.filmstrip.result_rx.try_recv() {
            self.filmstrip.pending.remove(&index);
            if gen != self.filmstrip.generation {
                continue;
            }
            if uploads >= FILMSTRIP_MAX_UPLOADS_PER_FRAME {
                break;
            }
            if frame.width == 0 || frame.height == 0 || frame.rgba.is_empty() {
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

    pub(super) fn render_filmstrip(&mut self, ctx: &egui::Context) {
        if self.sequence.entries.len() <= 1 {
            return;
        }

        let total = self.sequence.entries.len();
        let current = self.current_index;
        let item_w = FILMSTRIP_THUMB_SIZE + FILMSTRIP_SPACING;
        let total_content_w = total as f32 * item_w + FILMSTRIP_SPACING;

        egui::TopBottomPanel::bottom("filmstrip_panel")
            .exact_height(FILMSTRIP_PANEL_HEIGHT)
            .show(ctx, |ui| {
                let panel_bg = if ui.visuals().dark_mode {
                    egui::Color32::from_gray(30)
                } else {
                    egui::Color32::from_gray(220)
                };
                ui.painter()
                    .rect_filled(ui.max_rect(), 0.0, panel_bg);

                let should_scroll = self.filmstrip.scroll_to_current;
                self.filmstrip.scroll_to_current = false;

                let scroll_output = egui::ScrollArea::horizontal()
                    .id_salt("filmstrip_scroll")
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(FILMSTRIP_SPACING, 0.0);
                        ui.set_min_width(total_content_w);

                        let viewport_left = ui.clip_rect().left();
                        let viewport_right = ui.clip_rect().right();
                        let content_left = ui.min_rect().left();

                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = FILMSTRIP_SPACING;

                            for idx in 0..total {
                                let item_left =
                                    content_left + idx as f32 * item_w;
                                let item_right = item_left + FILMSTRIP_THUMB_SIZE;

                                let in_visible_range = item_right
                                    >= viewport_left - FILMSTRIP_OVERSCAN as f32 * item_w
                                    && item_left
                                        <= viewport_right
                                            + FILMSTRIP_OVERSCAN as f32 * item_w;

                                if !in_visible_range && !(should_scroll && idx == current) {
                                    // Allocate space but don't render
                                    ui.allocate_exact_size(
                                        egui::vec2(FILMSTRIP_THUMB_SIZE, FILMSTRIP_THUMB_SIZE),
                                        egui::Sense::hover(),
                                    );
                                    continue;
                                }

                                // Request decode if not loaded and not pending
                                if !self.filmstrip.thumbnails.contains_key(&idx)
                                    && !self.filmstrip.pending.contains(&idx)
                                {
                                    if let Some(path) = self.sequence.entries.get(idx).cloned() {
                                        let tx = self.filmstrip.result_tx.clone();
                                        let gen = self.filmstrip.generation;
                                        self.filmstrip.pending.insert(idx);
                                        rayon::spawn(move || {
                                            if let Ok(frame) =
                                                loader::decode_preview_frame_with_priority(
                                                    &path,
                                                    FILMSTRIP_DECODE_MAX_SIDE,
                                                    loader::DecodePriority::Background,
                                                )
                                            {
                                                let _ = tx.send((idx, gen, frame));
                                            }
                                        });
                                    }
                                }

                                let is_current = idx == current;

                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(FILMSTRIP_THUMB_SIZE, FILMSTRIP_THUMB_SIZE),
                                    egui::Sense::click(),
                                );

                                // Background
                                let bg_color = if is_current {
                                    if ui.visuals().dark_mode {
                                        egui::Color32::from_gray(50)
                                    } else {
                                        egui::Color32::from_gray(200)
                                    }
                                } else if response.hovered() {
                                    if ui.visuals().dark_mode {
                                        egui::Color32::from_gray(45)
                                    } else {
                                        egui::Color32::from_gray(210)
                                    }
                                } else {
                                    egui::Color32::TRANSPARENT
                                };
                                ui.painter().rect_filled(rect, 4.0, bg_color);

                                // Thumbnail image
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
                                    let image_rect =
                                        egui::Rect::from_center_size(rect.center(), draw_size);
                                    ui.painter().image(
                                        tex.id(),
                                        image_rect,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        egui::Color32::WHITE,
                                    );
                                } else {
                                    // Placeholder
                                    let placeholder_color = if ui.visuals().dark_mode {
                                        egui::Color32::from_gray(40)
                                    } else {
                                        egui::Color32::from_gray(200)
                                    };
                                    let inner = rect.shrink(4.0);
                                    ui.painter()
                                        .rect_filled(inner, 2.0, placeholder_color);
                                }

                                // Current image border highlight
                                if is_current {
                                    ui.painter().rect_stroke(
                                        rect,
                                        4.0,
                                        egui::Stroke::new(2.0, theme::COLOR_ACCENT),
                                        egui::StrokeKind::Outside,
                                    );
                                    if should_scroll {
                                        response.scroll_to_me(Some(egui::Align::Center));
                                    }
                                }

                                if response.clicked() {
                                    self.navigate_to(idx, ctx);
                                }
                            }
                        });
                    });

                // Request repaint if we have pending thumbnails
                if !self.filmstrip.pending.is_empty() {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }

                let _ = scroll_output;
            });
    }
}
