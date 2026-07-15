//! Native PDF viewer application built on eframe/egui.
//!
//! Background rendering via a dedicated worker thread, GPU-based rotation
//! through UV mapping, stale-texture display during zoom transitions, and
//! prefetching for fluid scrolling.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui;
use rust_i18n::t;

use super::render_worker::{
    RenderRequest, RenderWorker, SearchMatch, ThumbnailRequest, WorkerEvent,
};
use super::renderer::{PdfOpenError, PdfTextSegment};
use super::selection::{DragSelection, PageSelection};
use super::virtual_layout::{PageGeometry, PageRows, VariableRows};

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
const PREVIEW_RENDER_SIDE: u32 = 1536;

fn capped_render_size(width: f32, height: f32, max_side: u32) -> (u32, u32) {
    let width = width.max(1.0);
    let height = height.max(1.0);
    let factor = (max_side as f32 / width.max(height)).min(1.0);
    (
        (width * factor).round().max(1.0) as u32,
        (height * factor).round().max(1.0) as u32,
    )
}

fn preview_render_size(width: u32, height: u32) -> Option<(u32, u32)> {
    (width.max(height) > PREVIEW_RENDER_SIDE)
        .then(|| capped_render_size(width as f32, height as f32, PREVIEW_RENDER_SIDE))
}

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
    preview: bool,
}

impl PageTexture {
    /// RGBA memory footprint of this texture (4 bytes per pixel).
    fn byte_size(&self) -> usize {
        self.render_w as usize * self.render_h as usize * 4
    }
}

// ── Password prompt ─────────────────────────────────────────────────────────

#[derive(Default)]
pub(super) struct PasswordPrompt {
    input: String,
    /// Set to `true` after the user submits an incorrect password.
    wrong: bool,
    /// Only request focus once; re-requesting every frame breaks Enter handling.
    focus_requested: bool,
}

pub(super) enum DocumentStatus {
    Opening { password_attempted: bool },
    PasswordRequired(PasswordPrompt),
    LoadingMetadata,
    Ready,
    Failed(String),
}

// ── App ──────────────────────────────────────────────────────────────────────

pub struct PdfViewerApp {
    pub(super) worker_path: PathBuf,
    pub(super) worker: Option<RenderWorker>,
    pub(super) document_status: DocumentStatus,
    pending_password: Option<String>,

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
    scroll_to_page_fraction: Option<(u32, f32)>,
    current_page_fraction: f32,

    /// Effective zoom percentage (always reflects actual scale applied).
    pub(super) effective_zoom_pct: f32,

    // Texture cache (survives zoom/rotation — stale textures shown until replaced)
    textures: HashMap<u32, PageTexture>,
    /// Pages with in-flight render requests.
    pending: HashSet<u32>,
    render_failed: HashSet<u32>,
    needs_refinement: HashSet<u32>,
    render_generation: u64,
    viewport_anchor: u32,
    pub(super) thumbnail_textures: HashMap<u32, PageTexture>,
    pub(super) thumbnail_pending: HashSet<u32>,
    pub(super) thumbnail_failed: HashSet<u32>,
    pub(super) last_sidebar_scrolled_page: Option<u32>,
    pub(super) thumbnail_keyboard_focus: bool,
    page_rows: Option<(u32, u32, PageRows)>,
    pub(super) thumbnail_rows: Option<VariableRows>,
    /// Current total memory used by cached textures (tracked incrementally).
    cache_bytes: usize,

    pub(super) page_text: HashMap<u32, Vec<PdfTextSegment>>,
    pub(super) drag_selection: Option<DragSelection>,
    pub(super) selection: Option<PageSelection>,
    /// Whether to apply dark theme (set once at creation, applied on first frame).
    dark_mode: Option<bool>,
    /// First currently-visible page index (updated each frame by show_pages).
    visible_lo: Option<u32>,
    /// Last currently-visible page index (updated each frame by show_pages).
    visible_hi: u32,
    pub(super) search_active: bool,
    pub(super) search_query: String,
    pub(super) search_input_focus_requested: bool,
    pub(super) search_input_has_focus: bool,
    pub(super) search_results: Vec<SearchMatch>,
    pub(super) current_match_idx: usize,
    pub(super) search_generation: u64,
    pub(super) search_in_progress: bool,
    pub(super) last_searched_query: String,
}

impl PdfViewerApp {
    pub fn new(path: PathBuf, dark_mode: bool) -> Result<Self, String> {
        Ok(Self {
            worker_path: path,
            worker: None,
            document_status: DocumentStatus::Opening {
                password_attempted: false,
            },
            pending_password: None,
            total_pages: 0,
            page_sizes: Vec::new(),
            zoom: 1.0,
            zoom_mode: ZoomMode::FitWidth,
            page_layout: PdfPageLayout::OnePage,
            rotation: 0,
            current_page: 0,
            page_input: "1".into(),
            page_input_has_focus: false,
            scroll_to_page: None,
            scroll_to_page_fraction: None,
            current_page_fraction: 0.0,
            effective_zoom_pct: 100.0,
            textures: HashMap::new(),
            pending: HashSet::new(),
            render_failed: HashSet::new(),
            needs_refinement: HashSet::new(),
            render_generation: 0,
            viewport_anchor: 0,
            thumbnail_textures: HashMap::new(),
            thumbnail_pending: HashSet::new(),
            thumbnail_failed: HashSet::new(),
            last_sidebar_scrolled_page: None,
            thumbnail_keyboard_focus: false,
            page_rows: None,
            thumbnail_rows: None,
            cache_bytes: 0,
            page_text: HashMap::new(),
            drag_selection: None,
            selection: None,
            dark_mode: Some(dark_mode),
            visible_lo: None,
            visible_hi: 0,
            search_active: false,
            search_query: String::new(),
            search_input_focus_requested: false,
            search_input_has_focus: false,
            search_results: Vec::new(),
            current_match_idx: 0,
            search_generation: 0,
            search_in_progress: false,
            last_searched_query: String::new(),
        })
    }

    // ── Worker management ────────────────────────────────────────────────

    fn ensure_worker(&mut self, ctx: &egui::Context) {
        if self.worker.is_none() && matches!(self.document_status, DocumentStatus::Opening { .. }) {
            self.worker = Some(RenderWorker::spawn(
                self.worker_path.clone(),
                self.pending_password.take(),
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
                if let DocumentStatus::PasswordRequired(prompt) = &mut self.document_status {
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
            self.pending_password = Some(pwd);
            self.document_status = DocumentStatus::Opening {
                password_attempted: true,
            };
        }
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Opened {
                page_count,
                first_page_size,
            } => {
                self.total_pages = page_count;
                self.page_sizes = first_page_size
                    .map(|size| vec![size; page_count as usize])
                    .unwrap_or_default();
                self.document_status = if page_count <= 1 {
                    DocumentStatus::Ready
                } else {
                    DocumentStatus::LoadingMetadata
                };
                self.restore_document_state();
            }
            WorkerEvent::MetadataLoaded { page_sizes } => {
                if page_sizes.len() != self.total_pages as usize {
                    self.document_status = DocumentStatus::Failed(format!(
                        "PDF metadata count mismatch: expected {}, got {}",
                        self.total_pages,
                        page_sizes.len()
                    ));
                    return;
                }
                self.page_sizes = page_sizes;
                self.pending.clear();
                self.render_failed.clear();
                self.needs_refinement.extend(self.textures.keys().copied());
                self.thumbnail_pending.clear();
                self.thumbnail_textures.clear();
                self.thumbnail_failed.clear();
                self.page_rows = None;
                self.thumbnail_rows = None;
                self.last_sidebar_scrolled_page = None;
                self.drag_selection = None;
                self.selection = None;
                if self.scroll_to_page.is_none() {
                    self.scroll_to_page_fraction =
                        Some((self.current_page, self.current_page_fraction));
                }
                self.document_status = DocumentStatus::Ready;
            }
            WorkerEvent::RenderRequestsDropped {
                generation,
                page_indices,
            } => {
                if generation == self.render_generation {
                    for page_idx in page_indices {
                        self.pending.remove(&page_idx);
                    }
                }
            }
            WorkerEvent::Failed(PdfOpenError::PasswordRequired) => {
                self.worker = None;
                let wrong = matches!(
                    self.document_status,
                    DocumentStatus::Opening {
                        password_attempted: true
                    }
                );
                self.document_status = DocumentStatus::PasswordRequired(PasswordPrompt {
                    wrong,
                    ..Default::default()
                });
            }
            WorkerEvent::Failed(PdfOpenError::Other(error)) => {
                self.worker = None;
                self.document_status = DocumentStatus::Failed(error);
            }
        }
    }

    fn poll_results(&mut self, ctx: &egui::Context) {
        let events = match &self.worker {
            Some(worker) => worker.drain_events(),
            None => return,
        };

        for event in events {
            self.handle_worker_event(event);
        }

        let worker = match &self.worker {
            Some(worker) => worker,
            None => return,
        };

        let results = worker.drain_results(4);
        for r in results {
            if r.generation != self.render_generation {
                continue;
            }
            self.pending.remove(&r.page_idx);
            if let Some(error) = r.error {
                if r.preview
                    || (r.provisional && matches!(self.document_status, DocumentStatus::Ready))
                {
                    self.needs_refinement.insert(r.page_idx);
                } else {
                    self.render_failed.insert(r.page_idx);
                }
                log::error!("[PDF-VIEWER] page {} failed: {error}", r.page_idx);
                continue;
            }

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
                preview: r.preview,
            };
            if r.provisional && matches!(self.document_status, DocumentStatus::Ready) {
                self.needs_refinement.insert(r.page_idx);
            } else if !r.provisional && !r.preview {
                self.needs_refinement.remove(&r.page_idx);
            }
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
            if let Some(error) = r.error {
                self.thumbnail_failed.insert(r.page_idx);
                log::warn!("[PDF-VIEWER] thumbnail {} failed: {error}", r.page_idx);
                continue;
            }
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
                    preview: false,
                },
            );
            self.evict_thumbnail_cache();
        }
    }

    fn submit_render(
        &mut self,
        page_idx: u32,
        need_w: u32,
        need_h: u32,
        priority: u32,
        preview: bool,
    ) {
        if self.pending.contains(&page_idx) || self.render_failed.contains(&page_idx) {
            return;
        }
        if let Some(w) = &self.worker {
            if w.request(RenderRequest {
                page_idx,
                width: need_w,
                height: need_h,
                generation: self.render_generation,
                priority,
                provisional: matches!(self.document_status, DocumentStatus::LoadingMetadata),
                preview,
            }) {
                self.pending.insert(page_idx);
            }
        }
    }

    pub(super) fn submit_thumbnail(&mut self, page_idx: u32, width: u32, height: u32) {
        if matches!(self.document_status, DocumentStatus::LoadingMetadata) {
            return;
        }
        if self.thumbnail_pending.contains(&page_idx)
            || self.thumbnail_textures.contains_key(&page_idx)
            || self.thumbnail_failed.contains(&page_idx)
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
        capped_render_size(nw * scale * ppp, nh * scale * ppp, MAX_RENDER_SIDE as u32)
    }

    /// Returns `true` if the cached texture resolution is close enough to what
    /// we need (between 90 % and 200 %).  Outside that range the texture is
    /// either too blurry or wastefully large.
    fn texture_adequate(cached: &PageTexture, need_w: u32, need_h: u32) -> bool {
        if cached.preview || need_w == 0 || need_h == 0 {
            return false;
        }
        let rw = cached.render_w as f32 / need_w as f32;
        let rh = cached.render_h as f32 / need_h as f32;
        (0.9..=2.0).contains(&rw) && (0.9..=2.0).contains(&rh)
    }

    // ── Cache eviction ───────────────────────────────────────────────────

    fn evict_distant(&mut self, first_visible: Option<u32>, last_visible: u32) {
        let lo = self.current_page.saturating_sub(CACHE_RADIUS);
        let hi = self
            .current_page
            .saturating_add(CACHE_RADIUS)
            .min(self.total_pages.saturating_sub(1));
        let vis_lo = first_visible.unwrap_or(self.current_page);
        let vis_hi = last_visible.max(vis_lo);
        self.textures.retain(|&idx, tex| {
            if (idx >= lo && idx <= hi) || (idx >= vis_lo && idx <= vis_hi) {
                true
            } else {
                self.cache_bytes = self.cache_bytes.saturating_sub(tex.byte_size());
                false
            }
        });
        // Drop text-segment metadata for pages whose textures are no longer
        // cached; without this the per-page text cache grows unboundedly
        // during long browsing sessions on large documents.
        self.page_text
            .retain(|&idx, _| (idx >= lo && idx <= hi) || (idx >= vis_lo && idx <= vis_hi));

        // If still over budget, evict furthest pages from current_page first,
        // but never evict pages that are currently visible on screen — that
        // would cause a visible placeholder flash (render-evict cycle).
        if self.cache_bytes > TEXTURE_MEMORY_BUDGET {
            let mut pages: Vec<u32> = self.textures.keys().copied().collect();
            pages.sort_by_key(|&p| (p as i64 - self.current_page as i64).unsigned_abs());
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
        self.render_generation = self.render_generation.wrapping_add(1);
        self.pending.clear();
        self.render_failed.clear();
        self.thumbnail_failed.clear();
        self.page_rows = None;
        self.thumbnail_rows = None;
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
        if page != self.current_page {
            self.render_generation = self.render_generation.wrapping_add(1);
            self.pending.clear();
            self.render_failed.clear();
        }
        self.viewport_anchor = page;
        self.current_page = page;
        self.page_input = format!("{}", page + 1);
        self.scroll_to_page = Some(page);
        self.scroll_to_page_fraction = None;
        self.current_page_fraction = 0.0;
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
        let text_input_active = self.page_input_has_focus || self.search_input_has_focus;
        if self.thumbnail_keyboard_focus && !text_input_active {
            let (previous, next) = ctx.input_mut(|input| {
                if !input.modifiers.is_none() {
                    return (false, false);
                }
                let previous = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
                let next = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
                (previous, next)
            });
            if previous {
                self.go_to_page(self.current_page.saturating_sub(1));
            } else if next {
                self.go_to_page(
                    self.current_page
                        .saturating_add(1)
                        .min(self.total_pages.saturating_sub(1)),
                );
            }
        }

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

            if !text_input_active {
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
        if resp.clicked() || resp.drag_started() {
            self.thumbnail_keyboard_focus = false;
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
            Some(t) => {
                self.needs_refinement.contains(&idx) || !Self::texture_adequate(t, need_w, need_h)
            }
            None => true,
        };
        if needs_render {
            let preview_size =
                if self.textures.contains_key(&idx) || self.needs_refinement.contains(&idx) {
                    None
                } else {
                    preview_render_size(need_w, need_h)
                };
            let (request_w, request_h, preview) = preview_size
                .map(|(width, height)| (width, height, true))
                .unwrap_or((need_w, need_h, false));
            self.submit_render(idx, request_w, request_h, 0, preview);
        }

        if let Some(t) = self.textures.get(&idx) {
            Self::paint_page(ui.painter(), rect, t.texture.id(), self.rotation);
        } else {
            Self::paint_placeholder(ui.painter(), rect, idx, ui.visuals().dark_mode);
        }

        self.paint_search_highlights(ui.painter(), idx, rect);
        if matches!(self.document_status, DocumentStatus::Ready) {
            self.handle_page_selection(ui, resp, idx, rect);
        }
    }

    fn build_page_rows(&self, aw: f32, ah: f32, vertical_gap: f32) -> PageRows {
        let columns = if self.page_layout == PdfPageLayout::TwoPage {
            2
        } else {
            1
        };
        let page_aw = if columns == 2 {
            ((aw - 12.0) * 0.5).max(100.0)
        } else {
            aw
        };
        let pages = (0..self.total_pages)
            .map(|idx| {
                let scale = self.get_scale(idx, page_aw, ah);
                let (width, height) = self.display_size(idx, scale);
                PageGeometry {
                    scale,
                    size: egui::vec2(width, height),
                }
            })
            .collect();

        PageRows::new(pages, columns, 12.0, vertical_gap)
    }

    fn show_pages(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        rows: &PageRows,
        viewport: egui::Rect,
        content_width: f32,
    ) {
        let ppp = ctx.pixels_per_point();
        let mut first_visible: Option<u32> = None;
        let mut last_visible: u32 = 0;

        if let Some(row) = rows.visible_rows(viewport, 0).next() {
            let visible_pages = rows.pages_in_row(row);
            let anchor = if visible_pages.contains(&(self.current_page as usize)) {
                self.current_page
            } else {
                visible_pages.start as u32
            };
            if self.viewport_anchor != anchor {
                self.viewport_anchor = anchor;
                self.render_generation = self.render_generation.wrapping_add(1);
                self.pending.clear();
                self.render_failed.clear();
                if !self.page_input_has_focus {
                    self.current_page = anchor;
                    self.page_input = format!("{}", anchor + 1);
                }
            }
        }

        let origin = ui.max_rect().min.to_vec2();
        for row in rows.visible_rows(viewport, 1) {
            for page in rows.pages_in_row(row) {
                let Some(geometry) = rows.page(page) else {
                    continue;
                };
                let Some(relative_rect) = rows.page_rect(page, content_width) else {
                    continue;
                };
                let idx = page as u32;
                let rect = relative_rect.translate(origin);
                let response = ui.interact(
                    rect,
                    ui.id().with(("pdf_page", idx)),
                    egui::Sense::click_and_drag(),
                );
                self.show_page_slot(
                    ui,
                    idx,
                    rect,
                    &response,
                    geometry.scale,
                    ppp,
                    &mut first_visible,
                    &mut last_visible,
                );
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
            let cache_lo = self.current_page.saturating_sub(CACHE_RADIUS);
            let cache_hi =
                (self.current_page + CACHE_RADIUS).min(self.total_pages.saturating_sub(1));
            let pf_lo = fv.saturating_sub(PREFETCH_AHEAD).max(cache_lo);
            let pf_hi = (last_visible + PREFETCH_AHEAD).min(cache_hi);
            for pidx in pf_lo..=pf_hi {
                let Some(geometry) = rows.page(pidx as usize) else {
                    continue;
                };
                let (nw, nh) = self.needed_render_size(pidx, geometry.scale, ppp);
                let needs = match self.textures.get(&pidx) {
                    Some(t) if t.preview => false,
                    Some(t) => {
                        self.needs_refinement.contains(&pidx) || !Self::texture_adequate(t, nw, nh)
                    }
                    None => true,
                };
                if needs {
                    let preview_size = if self.textures.contains_key(&pidx)
                        || self.needs_refinement.contains(&pidx)
                    {
                        None
                    } else {
                        preview_render_size(nw, nh)
                    };
                    let (request_w, request_h, preview) = preview_size
                        .map(|(width, height)| (width, height, true))
                        .unwrap_or((nw, nh, false));
                    self.submit_render(
                        pidx,
                        request_w,
                        request_h,
                        100 + pidx.abs_diff(self.current_page),
                        preview,
                    );
                }
            }
        }

        // Publish visible range for eviction protection.
        self.visible_lo = first_visible;
        self.visible_hi = last_visible;
        self.current_page_fraction = rows
            .page_top(self.current_page as usize)
            .zip(rows.page(self.current_page as usize))
            .map(|(top, geometry)| {
                ((viewport.min.y - top) / geometry.size.y.max(1.0)).clamp(0.0, 1.0)
            })
            .unwrap_or(0.0);
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

        self.ensure_worker(ctx);
        self.poll_results(ctx);

        match &self.document_status {
            DocumentStatus::Opening { .. } => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                        ui.label(t!("pdfviewer.loading_document").to_string());
                    });
                });
                return;
            }
            DocumentStatus::PasswordRequired(_) => {
                egui::CentralPanel::default().show(ctx, |_ui| {});
                self.handle_password_dialog(ctx);
                return;
            }
            DocumentStatus::Failed(error) => {
                let error = error.clone();
                egui::CentralPanel::default().show(ctx, |ui| {
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
            DocumentStatus::LoadingMetadata | DocumentStatus::Ready => {}
        }

        self.handle_keyboard(ctx);
        self.handle_selection_shortcuts(ctx);
        if matches!(self.document_status, DocumentStatus::Ready) {
            self.handle_search_shortcuts(ctx);
        }

        egui::TopBottomPanel::top("pdf_toolbar").show(ctx, |ui| {
            self.show_toolbar(ui);
        });

        if matches!(self.document_status, DocumentStatus::LoadingMetadata) {
            egui::TopBottomPanel::top("pdf_loading_status").show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spinner();
                    ui.label(t!("pdfviewer.loading_pages").to_string());
                });
            });
        }

        self.show_search_bar(ctx);
        self.show_sidebar(ctx);

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

            let key = (aw.to_bits(), ah.to_bits());
            let rows = match self.page_rows.take() {
                Some((cached_aw, cached_ah, rows)) if (cached_aw, cached_ah) == key => rows,
                _ => self.build_page_rows(aw, ah, ui.spacing().item_spacing.y + 8.0),
            };
            let target_y = self
                .scroll_to_page
                .take()
                .and_then(|page| rows.page_top(page as usize))
                .or_else(|| {
                    self.scroll_to_page_fraction
                        .take()
                        .and_then(|(page, fraction)| {
                            rows.page_top(page as usize)
                                .zip(rows.page(page as usize))
                                .map(|(top, geometry)| {
                                    top + geometry.size.y * fraction.clamp(0.0, 1.0)
                                })
                        })
                });
            let mut scroll_area = egui::ScrollArea::both().auto_shrink([false, false]);
            if let Some(target_y) = target_y {
                scroll_area = scroll_area.vertical_scroll_offset(target_y);
            }
            scroll_area.show_viewport(ui, |ui, viewport| {
                let content_width = rows.content_width(viewport.width());
                ui.set_min_size(egui::vec2(content_width, rows.total_height()));
                self.show_pages(ui, ctx, &rows, viewport, content_width);
            });
            self.page_rows = Some((key.0, key.1, rows));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_cap_preserves_page_aspect_ratio() {
        let (width, height) = capped_render_size(8000.0, 4000.0, 4096);

        assert_eq!((width, height), (4096, 2048));
    }

    #[test]
    fn large_render_uses_bounded_preview() {
        assert_eq!(preview_render_size(3000, 2000), Some((1536, 1024)));
        assert_eq!(preview_render_size(1200, 800), None);
    }
}
