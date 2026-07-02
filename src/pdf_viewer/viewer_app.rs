//! Native PDF viewer application built on eframe/egui.
//!
//! Background rendering via a dedicated worker thread, GPU-based rotation
//! through UV mapping, stale-texture display during zoom transitions, and
//! prefetching for fluid scrolling.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui;
use rust_i18n::t;

use super::render_worker::{RenderRequest, RenderWorker, SearchMatch, ThumbnailRequest};
use super::renderer::{PdfRenderer, PdfTextSegment};
use super::selection::{DragSelection, PageSelection};

/// Pages beyond ±CACHE_RADIUS from the current view are evicted.
const CACHE_RADIUS: u32 = 3;

/// Number of pages to prefetch ahead/behind the visible range.
const PREFETCH_AHEAD: u32 = 1;

const THUMBNAIL_CACHE_LIMIT: usize = 256;

/// Maximum total memory (in bytes) for cached page textures.
/// When exceeded, furthest pages are evicted even if within CACHE_RADIUS.
/// 384 MB accommodates ~6 full-resolution pages at high zoom on HiDPI displays,
/// preventing render-evict cycles that cause visible flickering above 195%.
pub(super) const TEXTURE_MEMORY_BUDGET: usize = 384 * 1024 * 1024; // 384 MB

/// Hard cap on the longest side of a rendered page (pixels). Without this,
/// a heavily zoomed A0 page could allocate ~256 MB of RGBA per page; capping
/// at 4096 px keeps a worst-case page at ~64 MB. At zoom levels that would
/// exceed this, `texture_adequate()` (0.9–2.0×) lets the existing texture
/// stretch instead of triggering a re-render.
const MAX_RENDER_SIDE: f32 = 4096.0;

// ── Types ────────────────────────────────────────────────────────────────────

/// Zoom strategy.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ZoomMode {
    FitWidth,
    FitPage,
    Custom,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum PdfPageLayout {
    OnePage,
    TwoPage,
}

/// GPU-uploaded page texture with render-resolution metadata.
pub(super) struct PageTexture {
    pub(super) texture: egui::TextureHandle,
    pub(super) render_w: u32,
    pub(super) render_h: u32,
}

impl PageTexture {
    /// RGBA memory footprint of this texture (4 bytes per pixel).
    fn byte_size(&self) -> usize {
        self.render_w as usize * self.render_h as usize * 4
    }
}

// ── Password prompt ─────────────────────────────────────────────────────────

#[derive(Default)]
struct PasswordPrompt {
    input: String,
    /// Set to `true` after the user submits an incorrect password.
    wrong: bool,
    /// Only request focus once; re-requesting every frame breaks Enter handling.
    focus_requested: bool,
}

// ── App ──────────────────────────────────────────────────────────────────────

pub struct PdfViewerApp {
    pub(super) worker_path: PathBuf,
    pub(super) worker: Option<RenderWorker>,

    pub(super) total_pages: u32,
    /// Natural (unrotated) page sizes in DIP.
    pub(super) page_sizes: Vec<(f32, f32)>,

    // View state
    pub(super) zoom: f32,
    pub(super) zoom_mode: ZoomMode,
    pub(super) page_layout: PdfPageLayout,
    pub(super) rotation: u16, // 0 | 90 | 180 | 270

    // Navigation
    pub(super) current_page: u32,
    pub(super) page_input: String,
    pub(super) page_input_has_focus: bool,
    pub(super) scroll_to_page: Option<u32>,

    /// Effective zoom percentage (always reflects actual scale applied).
    pub(super) effective_zoom_pct: f32,

    // Texture cache (survives zoom/rotation — stale textures shown until replaced)
    textures: HashMap<u32, PageTexture>,
    /// Pages with in-flight render requests.
    pending: HashSet<u32>,
    pub(super) thumbnail_textures: HashMap<u32, PageTexture>,
    pub(super) thumbnail_pending: HashSet<u32>,
    pub(super) last_sidebar_scrolled_page: Option<u32>,
    /// Current total memory used by cached textures (tracked incrementally).
    cache_bytes: usize,

    pub(super) page_text: HashMap<u32, Vec<PdfTextSegment>>,
    pub(super) drag_selection: Option<DragSelection>,
    pub(super) selection: Option<PageSelection>,
    /// Error from the render worker (init failure).
    pub(super) worker_error: Option<String>,
    /// Whether to apply dark theme (set once at creation, applied on first frame).
    dark_mode: Option<bool>,
    /// First currently-visible page index (updated each frame by show_pages).
    visible_lo: Option<u32>,
    /// Last currently-visible page index (updated each frame by show_pages).
    visible_hi: u32,
    /// Active password prompt (present when the PDF is encrypted and no password has been confirmed yet).
    password_prompt: Option<PasswordPrompt>,
    /// The password successfully used to open this document.
    confirmed_password: Option<String>,

    pub(super) search_active: bool,
    pub(super) search_query: String,
    pub(super) search_input_focus_requested: bool,
    pub(super) search_results: Vec<SearchMatch>,
    pub(super) current_match_idx: usize,
    pub(super) search_generation: u64,
    pub(super) search_in_progress: bool,
    pub(super) last_searched_query: String,
}

impl PdfViewerApp {
    pub fn new(path: PathBuf, dark_mode: bool) -> Result<Self, String> {
        let (page_sizes, password_prompt) = match PdfRenderer::open(&path, None) {
            Ok(renderer) => (renderer.page_sizes().to_vec(), None),
            Err(e) if e.is_password_required() => (vec![], Some(PasswordPrompt::default())),
            Err(e) => return Err(e.to_string()),
        };
        let total_pages = page_sizes.len() as u32;

        let mut app = Self {
            worker_path: path,
            worker: None,
            total_pages,
            page_sizes,
            zoom: 1.0,
            zoom_mode: ZoomMode::FitWidth,
            page_layout: PdfPageLayout::OnePage,
            rotation: 0,
            current_page: 0,
            page_input: "1".into(),
            page_input_has_focus: false,
            scroll_to_page: None,
            effective_zoom_pct: 100.0,
            textures: HashMap::new(),
            pending: HashSet::new(),
            thumbnail_textures: HashMap::new(),
            thumbnail_pending: HashSet::new(),
            last_sidebar_scrolled_page: None,
            cache_bytes: 0,
            page_text: HashMap::new(),
            drag_selection: None,
            selection: None,
            worker_error: None,
            dark_mode: Some(dark_mode),
            visible_lo: None,
            visible_hi: 0,
            password_prompt,
            confirmed_password: None,
            search_active: false,
            search_query: String::new(),
            search_input_focus_requested: false,
            search_results: Vec::new(),
            current_match_idx: 0,
            search_generation: 0,
            search_in_progress: false,
            last_searched_query: String::new(),
        };
        app.restore_document_state();
        Ok(app)
    }

    // ── Worker management ────────────────────────────────────────────────

    fn ensure_worker(&mut self, ctx: &egui::Context) {
        if self.worker.is_none() {
            self.worker = Some(RenderWorker::spawn(
                self.worker_path.clone(),
                self.confirmed_password.clone(),
                ctx.clone(),
            ));
        }
    }

    /// Show the password-entry dialog.
    ///
    /// If the user submits a password, this method tries to open the PDF with
    /// it and — on success — populates `page_sizes`, `total_pages`, and
    /// `confirmed_password` so the normal render path can proceed.
    fn handle_password_dialog(&mut self, ctx: &egui::Context) {
        let mut submitted_password: Option<String> = None;
        let mut cancelled = false;

        egui::Window::new(t!("pdfviewer.password_title").to_string())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let Some(prompt) = &mut self.password_prompt {
                    ui.label(t!("pdfviewer.password_prompt").to_string());

                    if prompt.wrong {
                        ui.colored_label(
                            egui::Color32::RED,
                            t!("pdfviewer.password_wrong").to_string(),
                        );
                    }

                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut prompt.input)
                            .password(true)
                            .desired_width(260.0)
                            .hint_text(t!("pdfviewer.password_hint").to_string()),
                    );
                    if !prompt.focus_requested {
                        resp.request_focus();
                        prompt.focus_requested = true;
                    }

                    let submit_with_enter =
                        resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(t!("pdfviewer.password_submit").to_string())
                            .clicked()
                            || submit_with_enter
                        {
                            submitted_password = Some(prompt.input.clone());
                        }
                        if ui
                            .button(t!("pdfviewer.password_cancel").to_string())
                            .clicked()
                        {
                            cancelled = true;
                        }
                    });
                }
            });

        if cancelled {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if let Some(pwd) = submitted_password {
            match PdfRenderer::open(&self.worker_path, Some(&pwd)) {
                Ok(renderer) => {
                    self.page_sizes = renderer.page_sizes().to_vec();
                    self.total_pages = self.page_sizes.len() as u32;
                    self.confirmed_password = Some(pwd);
                    self.password_prompt = None;
                    self.restore_document_state();
                }
                Err(e) if e.is_password_required() => {
                    if let Some(prompt) = &mut self.password_prompt {
                        prompt.wrong = true;
                        prompt.input.clear();
                        prompt.focus_requested = false;
                    }
                }
                Err(e) => {
                    self.worker_error = Some(e.to_string());
                    self.password_prompt = None;
                }
            }
        }
    }

    fn poll_results(&mut self, ctx: &egui::Context) {
        let worker = match &self.worker {
            Some(w) => w,
            None => return,
        };

        if let Some(err) = worker.take_init_error() {
            log::error!("[PDF-VIEWER] render worker failed: {err}");
            self.worker_error = Some(err);
            self.worker = None;
            return;
        }

        let results = worker.drain_results(4);
        for r in results {
            self.pending.remove(&r.page_idx);

            // Free the old texture BEFORE uploading the new one to avoid
            // momentary 2× peak memory for this page, which can push the
            // cache over budget and trigger eviction of visible pages.
            if let Some(old) = self.textures.remove(&r.page_idx) {
                self.cache_bytes = self.cache_bytes.saturating_sub(old.byte_size());
                // old TextureHandle is dropped here, freeing GPU memory.
            }

            let tex = ctx.load_texture(
                format!("pdf_p{}", r.page_idx),
                egui::ColorImage::from_rgba_unmultiplied(
                    [r.width as usize, r.height as usize],
                    &r.pixels,
                ),
                egui::TextureOptions::LINEAR,
            );
            let new_entry = PageTexture {
                texture: tex,
                render_w: r.width,
                render_h: r.height,
            };
            self.cache_bytes += new_entry.byte_size();
            self.textures.insert(r.page_idx, new_entry);
        }

        let thumbnail_results = worker.drain_thumbnail_results(2);

        // Receive eagerly-extracted text segments from the worker.
        for r in worker.drain_text_segment_results() {
            self.page_text.insert(r.page_idx, r.segments);
        }

        // Receive bounded-text results and finalise pending selections.
        for r in worker.drain_bounded_text_results() {
            self.receive_bounded_text(r.page_idx, r.text);
        }

        // Receive search results.
        self.poll_search_results();

        for r in thumbnail_results {
            self.thumbnail_pending.remove(&r.page_idx);
            let tex = ctx.load_texture(
                format!("pdf_thumb_p{}", r.page_idx),
                egui::ColorImage::from_rgba_unmultiplied(
                    [r.width as usize, r.height as usize],
                    &r.pixels,
                ),
                egui::TextureOptions::LINEAR,
            );
            self.thumbnail_textures.insert(
                r.page_idx,
                PageTexture {
                    texture: tex,
                    render_w: r.width,
                    render_h: r.height,
                },
            );
            self.evict_thumbnail_cache();
        }
    }

    fn submit_render(&mut self, page_idx: u32, need_w: u32, need_h: u32) {
        if self.pending.contains(&page_idx) {
            return;
        }
        if let Some(w) = &self.worker {
            w.request(RenderRequest {
                page_idx,
                width: need_w,
                height: need_h,
            });
            self.pending.insert(page_idx);
        }
    }

    pub(super) fn submit_thumbnail(&mut self, page_idx: u32, width: u32, height: u32) {
        if self.thumbnail_pending.contains(&page_idx)
            || self.thumbnail_textures.contains_key(&page_idx)
        {
            return;
        }
        if let Some(w) = &self.worker {
            if w.request_thumbnail(ThumbnailRequest {
                page_idx,
                width,
                height,
            }) {
                self.thumbnail_pending.insert(page_idx);
            }
        }
    }

    fn evict_thumbnail_cache(&mut self) {
        if self.thumbnail_textures.len() <= THUMBNAIL_CACHE_LIMIT {
            return;
        }

        let mut pages = self.thumbnail_textures.keys().copied().collect::<Vec<_>>();
        pages.sort_by_key(|&p| (p as i64 - self.current_page as i64).abs());
        while self.thumbnail_textures.len() > THUMBNAIL_CACHE_LIMIT {
            let Some(page) = pages.pop() else { break };
            self.thumbnail_textures.remove(&page);
        }
    }

    // ── Geometry ─────────────────────────────────────────────────────────

    pub(super) fn get_scale(&self, page_idx: u32, aw: f32, ah: f32) -> f32 {
        let (nw, nh) = self.page_sizes[page_idx as usize];
        let (rw, rh) = if !self.rotation.is_multiple_of(180) {
            (nh, nw)
        } else {
            (nw, nh)
        };
        match self.zoom_mode {
            ZoomMode::FitWidth => aw / rw,
            ZoomMode::FitPage => (aw / rw).min(ah / rh),
            ZoomMode::Custom => self.zoom,
        }
    }

    fn display_size(&self, page_idx: u32, scale: f32) -> (f32, f32) {
        let (nw, nh) = self.page_sizes[page_idx as usize];
        let (rw, rh) = if !self.rotation.is_multiple_of(180) {
            (nh, nw)
        } else {
            (nw, nh)
        };
        (rw * scale, rh * scale)
    }

    /// Pixel dimensions the renderer should produce (unrotated, DPI-scaled).
    fn needed_render_size(&self, page_idx: u32, scale: f32, ppp: f32) -> (u32, u32) {
        let (nw, nh) = self.page_sizes[page_idx as usize];
        (
            (nw * scale * ppp).clamp(1.0, MAX_RENDER_SIDE) as u32,
            (nh * scale * ppp).clamp(1.0, MAX_RENDER_SIDE) as u32,
        )
    }

    /// Returns `true` if the cached texture resolution is close enough to what
    /// we need (between 90 % and 200 %).  Outside that range the texture is
    /// either too blurry or wastefully large.
    fn texture_adequate(cached: &PageTexture, need_w: u32, need_h: u32) -> bool {
        if need_w == 0 || need_h == 0 {
            return false;
        }
        let rw = cached.render_w as f32 / need_w as f32;
        let rh = cached.render_h as f32 / need_h as f32;
        (0.9..=2.0).contains(&rw) && (0.9..=2.0).contains(&rh)
    }

    // ── Cache eviction ───────────────────────────────────────────────────

    fn evict_distant(&mut self, first_visible: Option<u32>, last_visible: u32) {
        let lo = self.current_page.saturating_sub(CACHE_RADIUS);
        let hi = (self.current_page + CACHE_RADIUS).min(self.total_pages.saturating_sub(1));
        self.textures.retain(|&idx, tex| {
            if idx >= lo && idx <= hi {
                true
            } else {
                self.cache_bytes = self.cache_bytes.saturating_sub(tex.byte_size());
                false
            }
        });
        // Drop text-segment metadata for pages whose textures are no longer
        // cached; without this the per-page text cache grows unboundedly
        // during long browsing sessions on large documents.
        self.page_text.retain(|&idx, _| idx >= lo && idx <= hi);

        // If still over budget, evict furthest pages from current_page first,
        // but never evict pages that are currently visible on screen — that
        // would cause a visible placeholder flash (render-evict cycle).
        if self.cache_bytes > TEXTURE_MEMORY_BUDGET {
            let vis_lo = first_visible.unwrap_or(self.current_page);
            let vis_hi = if last_visible > 0 {
                last_visible
            } else {
                vis_lo
            };
            let mut pages: Vec<u32> = self.textures.keys().copied().collect();
            pages.sort_by_key(|&p| {
                std::cmp::Reverse((p as i64 - self.current_page as i64).unsigned_abs())
            });
            while self.cache_bytes > TEXTURE_MEMORY_BUDGET {
                if let Some(victim) = pages.pop() {
                    // Never evict the current page or any currently-visible page
                    if victim >= vis_lo && victim <= vis_hi {
                        continue;
                    }
                    if let Some(tex) = self.textures.remove(&victim) {
                        self.cache_bytes = self.cache_bytes.saturating_sub(tex.byte_size());
                    }
                    self.page_text.remove(&victim);
                } else {
                    break;
                }
            }
        }
    }

    // ── GPU-rotated painting ─────────────────────────────────────────────

    /// Paint a page texture with rotation handled entirely in UV coordinates —
    /// zero CPU pixel manipulation.
    pub(super) fn paint_page(
        painter: &egui::Painter,
        rect: egui::Rect,
        tex_id: egui::TextureId,
        rotation: u16,
    ) {
        let mut mesh = egui::Mesh::with_texture(tex_id);
        let c = egui::Color32::WHITE;
        let p = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
        // UV corners rotated to match the desired display rotation.
        let uv: [(f32, f32); 4] = match rotation {
            90 => [(0.0, 1.0), (0.0, 0.0), (1.0, 0.0), (1.0, 1.0)],
            180 => [(1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0)],
            270 => [(1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)],
            _ => [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
        };
        for i in 0..4 {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: p[i],
                uv: egui::pos2(uv[i].0, uv[i].1),
                color: c,
            });
        }
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        painter.add(egui::Shape::mesh(mesh));
    }

    fn paint_placeholder(painter: &egui::Painter, rect: egui::Rect, idx: u32, dark_mode: bool) {
        let bg = if dark_mode {
            egui::Color32::from_gray(40)
        } else {
            egui::Color32::from_gray(220)
        };
        let text_color = if dark_mode {
            egui::Color32::from_gray(140)
        } else {
            egui::Color32::from_gray(100)
        };
        painter.rect_filled(rect, 0.0, bg);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            t!("pdfviewer.page", page = idx + 1).to_string(),
            egui::FontId::proportional(18.0),
            text_color,
        );
    }

    // ── View helpers (called from toolbar.rs) ────────────────────────────

    /// Clear pending set so visible pages can be re-requested at the new scale.
    pub(super) fn on_view_changed(&mut self) {
        self.pending.clear();
    }

    pub(super) fn set_page_layout(&mut self, layout: PdfPageLayout) {
        if self.page_layout != layout {
            self.page_layout = layout;
            self.scroll_to_page = Some(self.current_page);
            self.on_view_changed();
        }
    }

    pub(super) fn zoom_in(&mut self) {
        self.zoom_mode = ZoomMode::Custom;
        self.zoom = (self.zoom * 1.25).min(5.0);
        self.on_view_changed();
    }

    pub(super) fn zoom_out(&mut self) {
        self.zoom_mode = ZoomMode::Custom;
        self.zoom = (self.zoom / 1.25).max(0.1);
        self.on_view_changed();
    }

    pub(super) fn rotate_cw(&mut self) {
        self.rotation = (self.rotation + 90) % 360;
        self.on_view_changed();
    }

    pub(super) fn rotate_ccw(&mut self) {
        self.rotation = (self.rotation + 270) % 360;
        self.on_view_changed();
    }

    pub(super) fn go_to_page(&mut self, page: u32) {
        let page = page.min(self.total_pages.saturating_sub(1));
        self.current_page = page;
        self.page_input = format!("{}", page + 1);
        self.scroll_to_page = Some(page);
    }

    pub(super) fn prev_page(&mut self) {
        if self.current_page == 0 {
            return;
        }

        if self.page_layout == PdfPageLayout::TwoPage {
            let spread_start = self.current_page.saturating_sub(self.current_page % 2);
            self.go_to_page(spread_start.saturating_sub(2));
        } else {
            self.go_to_page(self.current_page - 1);
        }
    }

    pub(super) fn next_page(&mut self) {
        if self.current_page + 1 >= self.total_pages {
            return;
        }

        if self.page_layout == PdfPageLayout::TwoPage {
            let spread_start = self.current_page.saturating_sub(self.current_page % 2);
            self.go_to_page((spread_start + 2).min(self.total_pages.saturating_sub(1)));
        } else {
            self.go_to_page(self.current_page + 1);
        }
    }

    // ── Keyboard ─────────────────────────────────────────────────────────

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            if i.events.iter().any(|e| matches!(e, egui::Event::Text(_))) {
                return;
            }
            if i.modifiers.ctrl {
                if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
                    self.zoom_in();
                }
                if i.key_pressed(egui::Key::Minus) {
                    self.zoom_out();
                }
                if i.key_pressed(egui::Key::Num0) {
                    self.zoom = 1.0;
                    self.zoom_mode = ZoomMode::Custom;
                    self.on_view_changed();
                }

                let dy = i.smooth_scroll_delta.y;
                if dy > 1.0 {
                    self.zoom_in();
                } else if dy < -1.0 {
                    self.zoom_out();
                }
            }

            if i.key_pressed(egui::Key::R) && !i.modifiers.ctrl {
                if i.modifiers.shift {
                    self.rotate_ccw();
                } else {
                    self.rotate_cw();
                }
            }

            if i.key_pressed(egui::Key::PageUp) {
                self.prev_page();
            }
            if i.key_pressed(egui::Key::PageDown) {
                self.next_page();
            }
            if i.key_pressed(egui::Key::Home) && i.modifiers.ctrl {
                self.go_to_page(0);
            }
            if i.key_pressed(egui::Key::End) && i.modifiers.ctrl {
                self.go_to_page(self.total_pages.saturating_sub(1));
            }
        });
    }

    // ── Page layout in ScrollArea ────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn show_page_slot(
        &mut self,
        ui: &mut egui::Ui,
        idx: u32,
        rect: egui::Rect,
        resp: &egui::Response,
        scale: f32,
        ppp: f32,
        first_visible: &mut Option<u32>,
        last_visible: &mut u32,
    ) {
        if self.scroll_to_page == Some(idx) {
            resp.scroll_to_me(Some(egui::Align::TOP));
            self.scroll_to_page = None;
        }

        if !ui.is_rect_visible(rect) {
            return;
        }

        if first_visible.is_none() {
            *first_visible = Some(idx);
        }
        *last_visible = idx;

        let (need_w, need_h) = self.needed_render_size(idx, scale, ppp);
        let needs_render = match self.textures.get(&idx) {
            Some(t) => !Self::texture_adequate(t, need_w, need_h),
            None => true,
        };
        if needs_render {
            self.submit_render(idx, need_w, need_h);
        }

        if let Some(t) = self.textures.get(&idx) {
            Self::paint_page(ui.painter(), rect, t.texture.id(), self.rotation);
        } else {
            Self::paint_placeholder(ui.painter(), rect, idx, ui.visuals().dark_mode);
        }

        self.paint_search_highlights(ui.painter(), idx, rect);
        self.handle_page_selection(ui, resp, idx, rect);
    }

    fn show_pages(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, aw: f32, ah: f32) {
        let ppp = ctx.pixels_per_point();
        let mut first_visible: Option<u32> = None;
        let mut last_visible: u32 = 0;

        match self.page_layout {
            PdfPageLayout::OnePage => {
                for idx in 0..self.total_pages {
                    self.show_vertical_page(
                        ui,
                        idx,
                        aw,
                        ah,
                        ppp,
                        &mut first_visible,
                        &mut last_visible,
                    );
                }
            }
            PdfPageLayout::TwoPage => {
                let mut idx = 0;
                while idx < self.total_pages {
                    self.show_two_page_spread(
                        ui,
                        idx,
                        aw,
                        ah,
                        ppp,
                        &mut first_visible,
                        &mut last_visible,
                    );
                    idx += 2;
                }
            }
        }

        // Update current-page indicator from scroll position
        if let Some(fv) = first_visible {
            if self.scroll_to_page.is_none() && !self.page_input_has_focus {
                let page = if self.page_layout == PdfPageLayout::TwoPage
                    && self.current_page >= fv
                    && self.current_page <= last_visible
                {
                    self.current_page
                } else {
                    fv
                };
                self.current_page = page;
                self.page_input = format!("{}", page + 1);
            }

            // Prefetch adjacent pages for smooth scrolling, clamped to
            // CACHE_RADIUS so prefetched textures are not immediately evicted.
            let prefetch_aw = if self.page_layout == PdfPageLayout::TwoPage {
                ((aw - 12.0) * 0.5).max(100.0)
            } else {
                aw
            };
            let scale0 = self.get_scale(fv, prefetch_aw, ah);
            let cache_lo = self.current_page.saturating_sub(CACHE_RADIUS);
            let cache_hi =
                (self.current_page + CACHE_RADIUS).min(self.total_pages.saturating_sub(1));
            let pf_lo = fv.saturating_sub(PREFETCH_AHEAD).max(cache_lo);
            let pf_hi = (last_visible + PREFETCH_AHEAD).min(cache_hi);
            for pidx in pf_lo..=pf_hi {
                let (nw, nh) = self.needed_render_size(pidx, scale0, ppp);
                let needs = match self.textures.get(&pidx) {
                    Some(t) => !Self::texture_adequate(t, nw, nh),
                    None => true,
                };
                if needs {
                    self.submit_render(pidx, nw, nh);
                }
            }
        }

        // Publish visible range for eviction protection.
        self.visible_lo = first_visible;
        self.visible_hi = last_visible;
    }

    #[allow(clippy::too_many_arguments)]
    fn show_vertical_page(
        &mut self,
        ui: &mut egui::Ui,
        idx: u32,
        aw: f32,
        ah: f32,
        ppp: f32,
        first_visible: &mut Option<u32>,
        last_visible: &mut u32,
    ) {
        let scale = self.get_scale(idx, aw, ah);
        let (dw, dh) = self.display_size(idx, scale);
        let indent = ((ui.available_width() - dw) / 2.0).max(0.0);

        ui.horizontal(|ui| {
            ui.add_space(indent);
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(dw, dh), egui::Sense::click_and_drag());
            self.show_page_slot(
                ui,
                idx,
                rect,
                &resp,
                scale,
                ppp,
                first_visible,
                last_visible,
            );
        });
        ui.add_space(8.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn show_two_page_spread(
        &mut self,
        ui: &mut egui::Ui,
        left: u32,
        aw: f32,
        ah: f32,
        ppp: f32,
        first_visible: &mut Option<u32>,
        last_visible: &mut u32,
    ) {
        let gap = 12.0;
        let page_aw = ((aw - gap) * 0.5).max(100.0);
        let right = left + 1;
        let pages = if right < self.total_pages {
            vec![left, right]
        } else {
            vec![left]
        };
        let layout = pages
            .iter()
            .map(|&idx| {
                let scale = self.get_scale(idx, page_aw, ah);
                let (dw, dh) = self.display_size(idx, scale);
                (idx, scale, dw, dh)
            })
            .collect::<Vec<_>>();
        let total_w = layout.iter().map(|(_, _, dw, _)| *dw).sum::<f32>()
            + gap * layout.len().saturating_sub(1) as f32;
        let indent = ((ui.available_width() - total_w) / 2.0).max(0.0);

        ui.horizontal(|ui| {
            ui.add_space(indent);
            for (pos, (idx, scale, dw, dh)) in layout.iter().copied().enumerate() {
                if pos > 0 {
                    ui.add_space(gap);
                }
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(dw, dh), egui::Sense::click_and_drag());
                self.show_page_slot(
                    ui,
                    idx,
                    rect,
                    &resp,
                    scale,
                    ppp,
                    first_visible,
                    last_visible,
                );
            }
        });
        ui.add_space(8.0);
    }
}

// ── eframe::App ──────────────────────────────────────────────────────────────

impl eframe::App for PdfViewerApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_document_state();
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Apply theme on first frame (cc.set_visuals in creator can be
        // overridden by the platform integration).
        if let Some(dark) = self.dark_mode.take() {
            if dark {
                ctx.set_visuals(egui::Visuals::dark());
            } else {
                ctx.set_visuals(egui::Visuals::light());
            }

            // Apply dark/light title bar on the native Windows decoration.
            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(wh) = handle.as_raw() {
                    let hwnd = windows::Win32::Foundation::HWND(wh.hwnd.get() as _);
                    crate::infrastructure::windows::center_window_on_primary_monitor(hwnd);
                    crate::infrastructure::windows::window_corners::apply_dark_title_bar(
                        hwnd, dark,
                    );
                }
            }

            // Reveal the window once the first frame is ready. The viewport
            // starts hidden (.with_visible(false)) to avoid startup flashing
            // before the viewer is ready to present its initial frame.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // ── Password prompt ──────────────────────────────────────────────────
        // Shown before any rendering; worker is not spawned until confirmed.
        if self.password_prompt.is_some() {
            egui::CentralPanel::default().show(ctx, |_ui| {});
            self.handle_password_dialog(ctx);
            return;
        }
        self.ensure_worker(ctx);
        self.poll_results(ctx);
        self.handle_keyboard(ctx);
        self.handle_selection_shortcuts(ctx);
        self.handle_search_shortcuts(ctx);

        egui::TopBottomPanel::top("pdf_toolbar").show(ctx, |ui| {
            self.show_toolbar(ui);
        });

        self.show_search_bar(ctx);
        self.show_sidebar(ctx);

        if let Some(err) = &self.worker_error {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new(err)
                            .color(egui::Color32::RED)
                            .size(16.0),
                    );
                });
            });
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let aw = (ui.available_width() - 20.0).max(100.0);
            let ah = ui.available_height().max(100.0);

            if self.total_pages > 0 {
                let zoom_page = self.current_page.min(self.total_pages.saturating_sub(1));
                let zoom_aw = if self.page_layout == PdfPageLayout::TwoPage {
                    ((aw - 12.0) * 0.5).max(100.0)
                } else {
                    aw
                };
                self.effective_zoom_pct = self.get_scale(zoom_page, zoom_aw, ah) * 100.0;
            }

            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.show_pages(ui, ctx, aw, ah);
                });
        });

        self.evict_distant(self.visible_lo, self.visible_hi);
    }
}

// ── Error fallback app ───────────────────────────────────────────────────────

/// Minimal app shown when the PDF fails to load.
pub(super) struct ErrorApp {
    pub message: String,
}

impl eframe::App for ErrorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Reveal window (started hidden to avoid wgpu init flicker).
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));

        egui::CentralPanel::default().show(ctx, |ui| {
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
