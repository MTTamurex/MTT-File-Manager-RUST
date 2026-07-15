//! Background PDF page rendering and text extraction worker.
//!
//! Owns a persistent Pdfium document handle and processes render and
//! text-extraction requests on a dedicated thread so the UI stays fluid
//! and never contends with the `thread_safe` pdfium mutex.
//!
//! Bounded-text requests (for clipboard copy) are handled via a separate
//! high-priority channel. Thumbnail work stays lower priority than visible
//! page renders.

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;

use super::ocr::OcrWord;
use super::renderer::{PdfOpenError, PdfTextBounds, PdfTextSegment};

const MAX_THUMBNAILS_PER_BATCH: usize = 8;
const MAX_PAGES_PER_BATCH: usize = 2;
const METADATA_PAGES_PER_BATCH: u32 = 32;

pub(super) struct RenderRequest {
    pub page_idx: u32,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub priority: u32,
    pub provisional: bool,
    pub preview: bool,
}

pub(super) struct RenderResult {
    pub page_idx: u32,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
    pub provisional: bool,
    pub preview: bool,
    pub error: Option<String>,
}

pub(super) struct ThumbnailRequest {
    pub page_idx: u32,
    pub width: u32,
    pub height: u32,
}

pub(super) struct ThumbnailResult {
    pub page_idx: u32,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub error: Option<String>,
}

pub(super) struct TextSegmentResult {
    pub page_idx: u32,
    pub segments: Vec<PdfTextSegment>,
}

pub(super) struct BoundedTextRequest {
    pub page_idx: u32,
    pub bounds: PdfTextBounds,
}

pub(super) struct BoundedTextResult {
    pub page_idx: u32,
    pub text: String,
}

pub(super) struct SearchRequest {
    pub query: String,
    pub generation: u64,
}

pub(super) struct SearchMatch {
    pub page_idx: u32,
    pub bounds: PdfTextBounds,
}

pub(super) struct SearchResult {
    pub query: String,
    pub generation: u64,
    pub matches: Vec<SearchMatch>,
}

pub(super) enum WorkerEvent {
    Opened {
        page_count: u32,
        first_page_size: Option<(f32, f32)>,
    },
    MetadataLoaded {
        page_sizes: Vec<(f32, f32)>,
    },
    RenderRequestsDropped {
        generation: u64,
        page_indices: Vec<u32>,
    },
    Failed(PdfOpenError),
}

pub(super) struct RenderWorker {
    tx: Sender<RenderRequest>,
    rx: Receiver<RenderResult>,
    thumbnail_tx: Sender<ThumbnailRequest>,
    thumbnail_rx: Receiver<ThumbnailResult>,
    text_seg_rx: Receiver<TextSegmentResult>,
    bounded_text_tx: Sender<BoundedTextRequest>,
    bounded_text_rx: Receiver<BoundedTextResult>,
    search_tx: Sender<SearchRequest>,
    search_res_rx: Receiver<SearchResult>,
    event_rx: Receiver<WorkerEvent>,
}

impl RenderWorker {
    /// Spawn the worker thread.
    pub fn spawn(path: PathBuf, password: Option<String>, repaint: egui::Context) -> Self {
        let (req_tx, req_rx) = crossbeam_channel::bounded(32);
        let (res_tx, res_rx) = crossbeam_channel::bounded(2);
        let (thumb_tx, thumb_rx) = crossbeam_channel::bounded(64);
        let (thumb_res_tx, thumb_res_rx) = crossbeam_channel::bounded(16);
        let (text_seg_tx, text_seg_rx) = crossbeam_channel::bounded(64);
        let (bt_req_tx, bt_req_rx) = crossbeam_channel::bounded(8);
        let (bt_res_tx, bt_res_rx) = crossbeam_channel::bounded(8);
        let (search_tx, search_rx) = crossbeam_channel::unbounded();
        let (search_res_tx, search_res_rx) = crossbeam_channel::bounded(4);
        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        std::thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || {
                worker_loop(
                    path,
                    password,
                    event_tx,
                    req_rx,
                    res_tx,
                    thumb_rx,
                    thumb_res_tx,
                    text_seg_tx,
                    bt_req_rx,
                    bt_res_tx,
                    search_rx,
                    search_res_tx,
                    repaint,
                )
            })
            .expect("spawn pdf-render thread");

        Self {
            tx: req_tx,
            rx: res_rx,
            thumbnail_tx: thumb_tx,
            thumbnail_rx: thumb_res_rx,
            text_seg_rx,
            bounded_text_tx: bt_req_tx,
            bounded_text_rx: bt_res_rx,
            search_tx,
            search_res_rx,
            event_rx,
        }
    }

    /// Submit a non-blocking render request.
    pub fn request(&self, req: RenderRequest) -> bool {
        self.tx.try_send(req).is_ok()
    }

    pub fn request_thumbnail(&self, req: ThumbnailRequest) -> bool {
        self.thumbnail_tx.try_send(req).is_ok()
    }

    /// Submit a bounded-text extraction request (for copy).
    pub fn request_bounded_text(&self, req: BoundedTextRequest) -> bool {
        self.bounded_text_tx.try_send(req).is_ok()
    }

    /// Submit a full-document search request.
    pub fn request_search(&self, req: SearchRequest) {
        let _ = self.search_tx.send(req);
    }

    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            out.push(event);
        }
        out
    }

    /// Drain up to `max` completed render results. Remaining results stay
    /// in the channel and are picked up on later frames; this avoids
    /// pushing many large `glTexImage2D` uploads through the Glow renderer
    /// in a single frame, which can degrade the GL kernel-mode driver and
    /// the OS compositor (DWM) under heavy churn.
    pub fn drain_results(&self, max: usize) -> Vec<RenderResult> {
        let mut out = Vec::with_capacity(max.min(8));
        for _ in 0..max {
            match self.rx.try_recv() {
                Ok(r) => out.push(r),
                Err(_) => break,
            }
        }
        out
    }

    pub fn drain_thumbnail_results(&self, max: usize) -> Vec<ThumbnailResult> {
        let mut out = Vec::with_capacity(max.min(8));
        for _ in 0..max {
            match self.thumbnail_rx.try_recv() {
                Ok(r) => out.push(r),
                Err(_) => break,
            }
        }
        out
    }

    /// Drain all completed text-segment results.
    pub fn drain_text_segment_results(&self) -> Vec<TextSegmentResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.text_seg_rx.try_recv() {
            out.push(r);
        }
        out
    }

    /// Drain all completed bounded-text results.
    pub fn drain_bounded_text_results(&self) -> Vec<BoundedTextResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.bounded_text_rx.try_recv() {
            out.push(r);
        }
        out
    }

    /// Drain all completed search results.
    pub fn drain_search_results(&self) -> Vec<SearchResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.search_res_rx.try_recv() {
            out.push(r);
        }
        out
    }
}

fn prioritize_render_requests(
    requests: HashMap<u32, RenderRequest>,
    current_generation: &mut u64,
) -> Vec<RenderRequest> {
    *current_generation = requests
        .values()
        .map(|request| request.generation)
        .max()
        .unwrap_or(*current_generation)
        .max(*current_generation);

    let mut requests = requests
        .into_values()
        .filter(|request| request.generation == *current_generation)
        .collect::<Vec<_>>();
    requests.sort_by_key(|request| (request.priority, request.page_idx));
    requests
}

fn page_size_without_loading(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
) -> Result<(f32, f32), String> {
    let rect = document
        .pages()
        .page_size(page_idx as pdfium_render::prelude::PdfPageIndex)
        .map_err(|error| error.to_string())?;
    Ok((rect.width().value, rect.height().value))
}

fn load_metadata_batch(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_count: u32,
    next_page: &mut u32,
    page_sizes: &mut Vec<(f32, f32)>,
) -> Result<bool, String> {
    let end = next_page
        .saturating_add(METADATA_PAGES_PER_BATCH)
        .min(page_count);
    while *next_page < end {
        page_sizes.push(page_size_without_loading(document, *next_page)?);
        *next_page += 1;
    }
    Ok(*next_page >= page_count)
}

#[allow(clippy::too_many_arguments)]
fn worker_loop(
    path: PathBuf,
    password: Option<String>,
    event_tx: Sender<WorkerEvent>,
    render_rx: Receiver<RenderRequest>,
    render_tx: Sender<RenderResult>,
    thumbnail_rx: Receiver<ThumbnailRequest>,
    thumbnail_tx: Sender<ThumbnailResult>,
    text_seg_tx: Sender<TextSegmentResult>,
    bt_rx: Receiver<BoundedTextRequest>,
    bt_tx: Sender<BoundedTextResult>,
    search_rx: Receiver<SearchRequest>,
    search_res_tx: Sender<SearchResult>,
    repaint: egui::Context,
) {
    // Keep a persistent Pdfium + document handle open for the lifetime of
    // this worker, avoiding repeated file open/close on every render.
    let pdfium = match super::renderer::pdfium() {
        Ok(p) => p,
        Err(err) => {
            log::error!("[PDF-RENDER] failed to init pdfium: {err}");
            let _ = event_tx.send(WorkerEvent::Failed(PdfOpenError::Other(err)));
            repaint.request_repaint();
            return;
        }
    };
    let document = match pdfium.load_pdf_from_file(&path, password.as_deref()) {
        Ok(document) => document,
        Err(err) => {
            let error = super::renderer::classify_open_error(err);
            if !error.is_password_required() {
                log::error!("[PDF-RENDER] failed to load document: {error}");
            }
            let _ = event_tx.send(WorkerEvent::Failed(error));
            repaint.request_repaint();
            return;
        }
    };

    let page_count = u32::from(document.pages().len());
    let first_page_size = if page_count == 0 {
        None
    } else {
        match page_size_without_loading(&document, 0) {
            Ok(size) => Some(size),
            Err(err) => {
                let _ = event_tx.send(WorkerEvent::Failed(PdfOpenError::Other(err)));
                repaint.request_repaint();
                return;
            }
        }
    };
    if event_tx
        .send(WorkerEvent::Opened {
            page_count,
            first_page_size,
        })
        .is_err()
    {
        return;
    }
    repaint.request_repaint();

    // Initialize WinRT MTA so Windows.Media.Ocr calls work on this thread.
    let _ = unsafe {
        windows::Win32::System::WinRT::RoInitialize(
            windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
        )
    };

    // Single cache for ALL page text segments (Pdfium or OCR).
    // Computed once on first render; re-sent on every subsequent render so
    // the UI-side page_text map is always up-to-date regardless of zoom
    // changes, eviction, or any other UI-side cache invalidation.
    let mut segment_cache: HashMap<u32, Vec<PdfTextSegment>> = HashMap::new();
    // OCR word data (with text) for scanned pages, used by bounded-text
    // extraction (Ctrl+C copy path).  Separate from segment_cache because it
    // carries the actual string content per word.
    let mut ocr_cache: HashMap<u32, Vec<OcrWord>> = HashMap::new();
    let mut search_text_cache: HashMap<u32, SearchText> = HashMap::new();
    let mut metadata_loaded = page_count <= 1;
    let mut metadata_started = false;
    let mut next_metadata_page = u32::from(page_count > 0);
    let mut page_sizes = first_page_size.into_iter().collect::<Vec<_>>();
    let mut render_generation = 0;

    loop {
        // Drain high-priority bounded-text requests before blocking.
        drain_bounded_text(
            &document,
            &bt_rx,
            &bt_tx,
            &text_seg_tx,
            &repaint,
            &mut segment_cache,
            &mut ocr_cache,
        );
        drain_search_requests(
            &document,
            &search_rx,
            &search_res_tx,
            &repaint,
            &mut search_text_cache,
        );

        if metadata_started && !metadata_loaded {
            match load_metadata_batch(
                &document,
                page_count,
                &mut next_metadata_page,
                &mut page_sizes,
            ) {
                Ok(true) => {
                    metadata_loaded = true;
                    if event_tx
                        .send(WorkerEvent::MetadataLoaded {
                            page_sizes: std::mem::take(&mut page_sizes),
                        })
                        .is_err()
                    {
                        return;
                    }
                    repaint.request_repaint();
                }
                Ok(false) => {
                    if render_rx.is_empty()
                        && bt_rx.is_empty()
                        && search_rx.is_empty()
                        && thumbnail_rx.is_empty()
                    {
                        continue;
                    }
                }
                Err(error) => {
                    let _ = event_tx.send(WorkerEvent::Failed(PdfOpenError::Other(error)));
                    repaint.request_repaint();
                    return;
                }
            }
        }

        // Prefer interactive work over thumbnail generation.
        crossbeam_channel::select_biased! {
            recv(bt_rx) -> msg => {
                let Ok(req) = msg else { return };
                handle_bounded_text(
                    &document,
                    &req,
                    &bt_tx,
                    &text_seg_tx,
                    &repaint,
                    &mut segment_cache,
                    &mut ocr_cache,
                );
            },
            recv(search_rx) -> msg => {
                let Ok(first) = msg else { return };
                let mut current = latest_search_request(first, &search_rx);
                while let Some(next) = handle_search(
                    &document,
                    &current,
                    &search_rx,
                    &search_res_tx,
                    &repaint,
                    &mut search_text_cache,
                ) {
                    current = latest_search_request(next, &search_rx);
                }
            },
            recv(render_rx) -> msg => {
                let first = match msg {
                    Ok(r) => r,
                    Err(_) => return, // channel closed — exit
                };

                // Drain + dedup: keep only the latest request per page
                let mut latest: HashMap<u32, RenderRequest> = HashMap::new();
                latest.insert(first.page_idx, first);
                while let Ok(r) = render_rx.try_recv() {
                    latest.insert(r.page_idx, r);
                }

                let mut latest = prioritize_render_requests(latest, &mut render_generation);
                if latest.len() > MAX_PAGES_PER_BATCH {
                    let dropped = latest.split_off(MAX_PAGES_PER_BATCH);
                    let _ = event_tx.send(WorkerEvent::RenderRequestsDropped {
                        generation: render_generation,
                        page_indices: dropped.into_iter().map(|request| request.page_idx).collect(),
                    });
                    repaint.request_repaint();
                }
                let latest_len = latest.len();
                for (position, req) in latest.into_iter().enumerate() {
                    // Prioritise bounded-text between page renders.
                    drain_bounded_text(
                        &document,
                        &bt_rx,
                        &bt_tx,
                        &text_seg_tx,
                        &repaint,
                        &mut segment_cache,
                        &mut ocr_cache,
                    );
                    drain_search_requests(
                        &document,
                        &search_rx,
                        &search_res_tx,
                        &repaint,
                        &mut search_text_cache,
                    );

                    let page_idx = req.page_idx;

                    let render_result = (|| -> Result<super::renderer::RenderedPage, String> {
                        let page = document
                            .pages()
                            .get(page_idx as pdfium_render::prelude::PdfPageIndex)
                            .map_err(|e| e.to_string())?;

                        let bitmap = page
                            .render(
                                req.width as pdfium_render::prelude::Pixels,
                                req.height as pdfium_render::prelude::Pixels,
                                None,
                            )
                            .map_err(|e| format!("RenderPage: {e}"))?;

                        Ok(super::renderer::RenderedPage {
                            width: bitmap.width() as u32,
                            height: bitmap.height() as u32,
                            pixels: bitmap.as_rgba_bytes(),
                        })
                    })();

                    match render_result {
                        Ok(p) => {
                            let _ = render_tx.send(RenderResult {
                                page_idx,
                                pixels: p.pixels,
                                width: p.width,
                                height: p.height,
                                generation: req.generation,
                                provisional: req.provisional,
                                preview: req.preview,
                                error: None,
                            });
                            repaint.request_repaint();

                            send_text_segments_if_ready(
                                &document,
                                page_idx,
                                &render_rx,
                                &bt_rx,
                                &search_rx,
                                metadata_loaded && position + 1 == latest_len,
                                &mut segment_cache,
                                &text_seg_tx,
                                &repaint,
                            );
                        }
                        Err(e) => {
                            log::error!("[PDF-RENDER] page {page_idx} failed: {e}");
                            let _ = render_tx.send(RenderResult {
                                page_idx,
                                pixels: Vec::new(),
                                width: 0,
                                height: 0,
                                generation: req.generation,
                                provisional: req.provisional,
                                preview: req.preview,
                                error: Some(e),
                            });
                            repaint.request_repaint();
                        }
                    }
                    metadata_started = true;
                }
            },
            recv(thumbnail_rx) -> msg => {
                let Ok(first) = msg else { return };
                handle_thumbnail_requests(&document, first, &thumbnail_rx, &thumbnail_tx, &repaint);
            },
        }
    }
}

// ── Text extraction helpers ──────────────────────────────────────────────────

fn handle_thumbnail_requests(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    first: ThumbnailRequest,
    rx: &Receiver<ThumbnailRequest>,
    tx: &Sender<ThumbnailResult>,
    repaint: &egui::Context,
) {
    let mut latest: HashMap<u32, ThumbnailRequest> = HashMap::new();
    latest.insert(first.page_idx, first);
    for _ in 1..MAX_THUMBNAILS_PER_BATCH {
        let Ok(r) = rx.try_recv() else { break };
        latest.insert(r.page_idx, r);
    }

    for (_, req) in latest {
        let result = (|| -> Result<ThumbnailResult, String> {
            let page = document
                .pages()
                .get(req.page_idx as pdfium_render::prelude::PdfPageIndex)
                .map_err(|e| e.to_string())?;
            let bitmap = page
                .render(
                    req.width as pdfium_render::prelude::Pixels,
                    req.height as pdfium_render::prelude::Pixels,
                    None,
                )
                .map_err(|e| format!("RenderThumbnail: {e}"))?;
            Ok(ThumbnailResult {
                page_idx: req.page_idx,
                pixels: bitmap.as_rgba_bytes(),
                width: bitmap.width() as u32,
                height: bitmap.height() as u32,
                error: None,
            })
        })();

        match result {
            Ok(result) => {
                let _ = tx.send(result);
                repaint.request_repaint();
            }
            Err(e) => {
                log::warn!("[PDF-RENDER] thumbnail {} failed: {e}", req.page_idx);
                let _ = tx.send(ThumbnailResult {
                    page_idx: req.page_idx,
                    pixels: Vec::new(),
                    width: 0,
                    height: 0,
                    error: Some(e),
                });
                repaint.request_repaint();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_text_segments_if_ready(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
    render_rx: &Receiver<RenderRequest>,
    bt_rx: &Receiver<BoundedTextRequest>,
    search_rx: &Receiver<SearchRequest>,
    allow_text_work: bool,
    segment_cache: &mut HashMap<u32, Vec<PdfTextSegment>>,
    text_seg_tx: &Sender<TextSegmentResult>,
    repaint: &egui::Context,
) {
    let can_extract =
        allow_text_work && render_rx.is_empty() && bt_rx.is_empty() && search_rx.is_empty();

    if can_extract {
        if let std::collections::hash_map::Entry::Vacant(entry) = segment_cache.entry(page_idx) {
            match extract_text_segments(document, page_idx) {
                Ok(segments) if !segments.is_empty() => {
                    entry.insert(segments);
                }
                Ok(_) => {
                    // Scanned-page OCR is intentionally left to explicit search.
                    // Running it during normal navigation blocks page/thumbnail rendering.
                }
                Err(e) => {
                    log::error!(
                        "[PDF-RENDER] text segment extraction for page {page_idx} failed: {e}"
                    );
                }
            }
        }
    }

    // Re-send cached segments on every render so UI-side page_text is refreshed
    // after zoom changes or UI-side cache eviction.
    if let Some(segments) = segment_cache.get(&page_idx) {
        let _ = text_seg_tx.send(TextSegmentResult {
            page_idx,
            segments: segments.clone(),
        });
        repaint.request_repaint();
    }
}

fn drain_bounded_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    rx: &Receiver<BoundedTextRequest>,
    tx: &Sender<BoundedTextResult>,
    text_seg_tx: &Sender<TextSegmentResult>,
    repaint: &egui::Context,
    segment_cache: &mut HashMap<u32, Vec<PdfTextSegment>>,
    ocr_cache: &mut HashMap<u32, Vec<OcrWord>>,
) {
    while let Ok(req) = rx.try_recv() {
        handle_bounded_text(
            document,
            &req,
            tx,
            text_seg_tx,
            repaint,
            segment_cache,
            ocr_cache,
        );
    }
}

fn handle_bounded_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    req: &BoundedTextRequest,
    tx: &Sender<BoundedTextResult>,
    text_seg_tx: &Sender<TextSegmentResult>,
    repaint: &egui::Context,
    segment_cache: &mut HashMap<u32, Vec<PdfTextSegment>>,
    ocr_cache: &mut HashMap<u32, Vec<OcrWord>>,
) {
    let pdfium_text =
        extract_bounded_text(document, req.page_idx, req.bounds).unwrap_or_else(|e| {
            log::warn!(
                "[PDF-RENDER] bounded text for page {} failed: {e}",
                req.page_idx
            );
            String::new()
        });

    // For scanned PDFs (no text layer) Pdfium returns empty; fall back to
    // words collected by Windows OCR that overlap the selection bounds.
    let text = if pdfium_text.is_empty() {
        let words = ensure_ocr_words(document, req.page_idx, ocr_cache);
        if !words.is_empty() && !segment_cache.contains_key(&req.page_idx) {
            let segments = words
                .iter()
                .map(|word| PdfTextSegment {
                    bounds: word.bounds,
                })
                .collect::<Vec<_>>();
            segment_cache.insert(req.page_idx, segments.clone());
            let _ = text_seg_tx.send(TextSegmentResult {
                page_idx: req.page_idx,
                segments,
            });
        }
        words
            .iter()
            .filter(|w| w.bounds.overlaps(&req.bounds))
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        pdfium_text
    };

    let _ = tx.send(BoundedTextResult {
        page_idx: req.page_idx,
        text,
    });
    repaint.request_repaint();
}

fn ensure_ocr_words<'a>(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
    ocr_cache: &'a mut HashMap<u32, Vec<OcrWord>>,
) -> &'a Vec<OcrWord> {
    ocr_cache.entry(page_idx).or_insert_with(|| {
        let page = match document
            .pages()
            .get(page_idx as pdfium_render::prelude::PdfPageIndex)
        {
            Ok(page) => page,
            Err(e) => {
                log::warn!("[PDF-OCR] cannot load page {page_idx} for selection OCR: {e}");
                return Vec::new();
            }
        };
        run_ocr_canonical_words(document, page_idx, page.width().value, page.height().value)
    })
}

/// Canonical OCR side length (pixels).  Represents a good trade-off between
/// recognition quality and runtime: long side ~200 DPI for a Letter page.
const OCR_CANONICAL_SIDE: u32 = 1500;

fn extract_text_segments(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
) -> Result<Vec<PdfTextSegment>, String> {
    let page = document
        .pages()
        .get(page_idx as pdfium_render::prelude::PdfPageIndex)
        .map_err(|e| e.to_string())?;
    let text = page.text().map_err(|e| format!("LoadText: {e}"))?;

    let mut segments = Vec::new();
    for character in text.chars().iter() {
        let Some(content) = character.unicode_string() else {
            continue;
        };
        if content.trim().is_empty() {
            continue;
        }
        let bounds = character
            .loose_bounds()
            .or_else(|_| character.tight_bounds())
            .map_err(|e| format!("GetCharBounds: {e}"))?;
        segments.push(PdfTextSegment {
            bounds: super::renderer::pdfium_rect_to_bounds(bounds),
        });
    }
    Ok(segments)
}

fn extract_bounded_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
    bounds: PdfTextBounds,
) -> Result<String, String> {
    let page = document
        .pages()
        .get(page_idx as pdfium_render::prelude::PdfPageIndex)
        .map_err(|e| e.to_string())?;
    let text = page.text().map_err(|e| format!("LoadText: {e}"))?;
    Ok(
        text.inside_rect(pdfium_render::prelude::PdfRect::new_from_values(
            bounds.bottom,
            bounds.left,
            bounds.top,
            bounds.right,
        )),
    )
}

// ── Search ────────────────────────────────────────────────────────────────────

struct SearchText {
    chars: Vec<char>,
    bounds: Vec<Option<PdfTextBounds>>,
    fallback_bounds: PdfTextBounds,
}

enum SearchRun {
    Complete(Vec<SearchMatch>),
    Interrupted(SearchRequest),
}

fn latest_search_request(first: SearchRequest, rx: &Receiver<SearchRequest>) -> SearchRequest {
    let mut latest = first;
    while let Ok(r) = rx.try_recv() {
        latest = r;
    }
    latest
}

fn drain_search_requests(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    rx: &Receiver<SearchRequest>,
    tx: &Sender<SearchResult>,
    repaint: &egui::Context,
    page_text_cache: &mut HashMap<u32, SearchText>,
) {
    while let Ok(first) = rx.try_recv() {
        let mut current = latest_search_request(first, rx);
        while let Some(next) = handle_search(document, &current, rx, tx, repaint, page_text_cache) {
            current = latest_search_request(next, rx);
        }
    }
}

fn handle_search(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    req: &SearchRequest,
    rx: &Receiver<SearchRequest>,
    tx: &Sender<SearchResult>,
    repaint: &egui::Context,
    page_text_cache: &mut HashMap<u32, SearchText>,
) -> Option<SearchRequest> {
    match perform_search(document, &req.query, rx, page_text_cache) {
        Ok(SearchRun::Complete(matches)) => {
            let _ = tx.send(SearchResult {
                query: req.query.clone(),
                generation: req.generation,
                matches,
            });
            repaint.request_repaint();
            None
        }
        Ok(SearchRun::Interrupted(next)) => Some(next),
        Err(e) => {
            log::warn!("[PDF-SEARCH] search failed: {e}");
            let _ = tx.send(SearchResult {
                query: req.query.clone(),
                generation: req.generation,
                matches: Vec::new(),
            });
            repaint.request_repaint();
            None
        }
    }
}

fn perform_search(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    query: &str,
    rx: &Receiver<SearchRequest>,
    page_text_cache: &mut HashMap<u32, SearchText>,
) -> Result<SearchRun, String> {
    if query.is_empty() {
        return Ok(SearchRun::Complete(Vec::new()));
    }

    let query_lower: Vec<char> = query.chars().map(lower_search_char).collect();
    if query_lower.is_empty() {
        return Ok(SearchRun::Complete(Vec::new()));
    }

    let page_count = document.pages().len();
    let mut all_matches = Vec::new();

    for page_idx_u16 in 0..page_count {
        let page_idx: u32 = page_idx_u16 as u32;
        match page_text_cache.entry(page_idx) {
            std::collections::hash_map::Entry::Occupied(entry) => {
                all_matches.extend(search_indexed_text(page_idx, entry.get(), &query_lower));
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let indexed = match extract_page_search_text(document, page_idx_u16, page_idx) {
                    Ok(indexed) => indexed,
                    Err(e) => {
                        log::warn!("[PDF-SEARCH] search: cannot index page {page_idx}: {e}");
                        continue;
                    }
                };
                let indexed = entry.insert(indexed);
                all_matches.extend(search_indexed_text(page_idx, indexed, &query_lower));
            }
        }

        if let Ok(next) = rx.try_recv() {
            return Ok(SearchRun::Interrupted(next));
        }
    }

    Ok(SearchRun::Complete(all_matches))
}

fn extract_page_search_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx_u16: pdfium_render::prelude::PdfPageIndex,
    page_idx: u32,
) -> Result<SearchText, String> {
    let page = document
        .pages()
        .get(page_idx_u16)
        .map_err(|e| e.to_string())?;

    let page_w = page.width().value;
    let page_h = page.height().value;
    let indexed = match page.text() {
        Ok(text) => indexed_text_from_pdf_text(&text, page_w, page_h),
        Err(_) => SearchText {
            chars: Vec::new(),
            bounds: Vec::new(),
            fallback_bounds: PdfTextBounds::from_points(0.0, page_w, page_h, 0.0),
        },
    };

    if indexed.chars.is_empty() {
        let words = run_ocr_canonical_words(document, page_idx, page_w, page_h);
        Ok(indexed_text_from_ocr_words(&words))
    } else {
        Ok(indexed)
    }
}

fn indexed_text_from_pdf_text(
    text: &pdfium_render::prelude::PdfPageText<'_>,
    page_w: f32,
    page_h: f32,
) -> SearchText {
    let chars = text
        .all()
        .chars()
        .map(lower_search_char)
        .collect::<Vec<_>>();
    let mut bounds = Vec::new();

    for character in text.chars().iter() {
        let Some(unicode) = character.unicode_string() else {
            continue;
        };
        let char_bounds = character
            .loose_bounds()
            .or_else(|_| character.tight_bounds())
            .ok()
            .map(super::renderer::pdfium_rect_to_bounds);
        for _ in unicode.chars() {
            bounds.push(char_bounds);
        }
    }

    if bounds.len() != chars.len() {
        bounds = vec![None; chars.len()];
    }

    SearchText {
        chars,
        bounds,
        fallback_bounds: PdfTextBounds::from_points(0.0, page_w, page_h, 0.0),
    }
}

fn indexed_text_from_ocr_words(words: &[OcrWord]) -> SearchText {
    let mut chars = Vec::new();
    let mut bounds = Vec::new();

    for word in words {
        let text = word.text.trim();
        if text.is_empty() {
            continue;
        }
        if !chars.is_empty() {
            chars.push(' ');
            bounds.push(None);
        }
        for ch in text.chars() {
            chars.push(lower_search_char(ch));
            bounds.push(Some(word.bounds));
        }
    }

    let fallback_bounds = if words.is_empty() {
        PdfTextBounds::from_points(0.0, 0.0, 0.0, 0.0)
    } else {
        let word_bounds = words.iter().map(|w| Some(w.bounds)).collect::<Vec<_>>();
        union_bounds(&word_bounds).unwrap_or_else(|| PdfTextBounds::from_points(0.0, 0.0, 0.0, 0.0))
    };

    SearchText {
        chars,
        bounds,
        fallback_bounds,
    }
}

fn search_indexed_text(
    page_idx: u32,
    indexed: &SearchText,
    query_lower: &[char],
) -> Vec<SearchMatch> {
    if indexed.chars.is_empty() || query_lower.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut start = 0;
    while start + query_lower.len() <= indexed.chars.len() {
        if indexed.chars[start..start + query_lower.len()] == query_lower[..] {
            let end = start + query_lower.len();
            let bounds =
                union_bounds(&indexed.bounds[start..end]).unwrap_or(indexed.fallback_bounds);
            matches.push(SearchMatch { page_idx, bounds });
            start += 1;
        } else {
            start += 1;
        }
    }

    matches
}

fn run_ocr_canonical_words(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
    page_w: f32,
    page_h: f32,
) -> Vec<OcrWord> {
    let scale = OCR_CANONICAL_SIDE as f32 / page_w.max(page_h);
    let ocr_w = ((page_w * scale) as u32).max(1);
    let ocr_h = ((page_h * scale) as u32).max(1);

    let result = (|| -> Result<(Vec<u8>, u32, u32), String> {
        let page = document
            .pages()
            .get(page_idx as pdfium_render::prelude::PdfPageIndex)
            .map_err(|e| e.to_string())?;
        let bm = page
            .render(
                ocr_w as pdfium_render::prelude::Pixels,
                ocr_h as pdfium_render::prelude::Pixels,
                None,
            )
            .map_err(|e| format!("OCR search render: {e}"))?;
        Ok((bm.as_rgba_bytes(), bm.width() as u32, bm.height() as u32))
    })();

    match result {
        Ok((pixels, w, h)) => {
            super::ocr::ocr_page_bitmap(&pixels, w, h, page_w, page_h).unwrap_or_default()
        }
        Err(e) => {
            log::warn!("[PDF-OCR] OCR render for page {page_idx} failed: {e}");
            Vec::new()
        }
    }
}

fn lower_search_char(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn union_bounds(bounds_slice: &[Option<PdfTextBounds>]) -> Option<PdfTextBounds> {
    let mut min_left = f32::INFINITY;
    let mut max_right = f32::NEG_INFINITY;
    let mut min_bottom = f32::INFINITY;
    let mut max_top = f32::NEG_INFINITY;
    let mut found = false;
    for bounds in bounds_slice {
        let Some(b) = bounds else {
            continue;
        };
        found = true;
        min_left = min_left.min(b.left);
        max_right = max_right.max(b.right);
        min_bottom = min_bottom.min(b.bottom);
        max_top = max_top.max(b.top);
    }
    found.then(|| PdfTextBounds::from_points(min_left, max_right, max_top, min_bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(page_idx: u32, generation: u64, priority: u32) -> RenderRequest {
        RenderRequest {
            page_idx,
            width: 100,
            height: 100,
            generation,
            priority,
            provisional: false,
            preview: false,
        }
    }

    #[test]
    fn newer_generation_discards_stale_render_work() {
        let mut requests = HashMap::new();
        requests.insert(1, request(1, 3, 0));
        requests.insert(2, request(2, 4, 50));
        let mut generation = 3;

        let scheduled = prioritize_render_requests(requests, &mut generation);

        assert_eq!(generation, 4);
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].page_idx, 2);
    }

    #[test]
    fn visible_pages_are_scheduled_before_prefetch() {
        let mut requests = HashMap::new();
        requests.insert(9, request(9, 2, 109));
        requests.insert(5, request(5, 2, 0));
        requests.insert(6, request(6, 2, 1));
        let mut generation = 2;

        let scheduled = prioritize_render_requests(requests, &mut generation);
        let pages = scheduled
            .into_iter()
            .map(|request| request.page_idx)
            .collect::<Vec<_>>();

        assert_eq!(pages, vec![5, 6, 9]);
    }
}
