//! Background PDF page rendering and text extraction worker.
//!
//! Owns a persistent Pdfium document handle and processes both render
//! and text-extraction requests on a dedicated thread so the UI stays
//! fluid and never contends with the `thread_safe` pdfium mutex.
//!
//! Text segments are extracted eagerly alongside the first render of
//! each page. Bounded-text requests (for clipboard copy) are handled
//! via a separate high-priority channel.

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::ocr::OcrWord;
use super::renderer::{PdfTextBounds, PdfTextSegment};

pub(super) struct RenderRequest {
    pub page_idx: u32,
    pub width: u32,
    pub height: u32,
}

pub(super) struct RenderResult {
    pub page_idx: u32,
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
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

pub(super) struct RenderWorker {
    tx: Sender<RenderRequest>,
    rx: Receiver<RenderResult>,
    text_seg_rx: Receiver<TextSegmentResult>,
    bounded_text_tx: Sender<BoundedTextRequest>,
    bounded_text_rx: Receiver<BoundedTextResult>,
    /// Set by the worker thread if PDF initialisation fails.
    init_error: Arc<std::sync::Mutex<Option<String>>>,
}

impl RenderWorker {
    /// Spawn the worker thread.
    pub fn spawn(path: PathBuf, password: Option<String>, repaint: egui::Context) -> Self {
        let (req_tx, req_rx) = crossbeam_channel::bounded(32);
        let (res_tx, res_rx) = crossbeam_channel::bounded(64);
        let (text_seg_tx, text_seg_rx) = crossbeam_channel::bounded(64);
        let (bt_req_tx, bt_req_rx) = crossbeam_channel::bounded(8);
        let (bt_res_tx, bt_res_rx) = crossbeam_channel::bounded(8);
        let init_error = Arc::new(std::sync::Mutex::new(None));
        let init_error_w = Arc::clone(&init_error);

        std::thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || {
                worker_loop(
                    path,
                    password,
                    req_rx,
                    res_tx,
                    text_seg_tx,
                    bt_req_rx,
                    bt_res_tx,
                    repaint,
                    init_error_w,
                )
            })
            .expect("spawn pdf-render thread");

        Self {
            tx: req_tx,
            rx: res_rx,
            text_seg_rx,
            bounded_text_tx: bt_req_tx,
            bounded_text_rx: bt_res_rx,
            init_error,
        }
    }

    /// Submit a non-blocking render request.
    pub fn request(&self, req: RenderRequest) {
        let _ = self.tx.send(req);
    }

    /// Submit a bounded-text extraction request (for copy).
    pub fn request_bounded_text(&self, req: BoundedTextRequest) {
        let _ = self.bounded_text_tx.send(req);
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

    /// Returns the initialisation error if the worker failed to start.
    pub fn take_init_error(&self) -> Option<String> {
        self.init_error.lock().ok()?.take()
    }
}

fn worker_loop(
    path: PathBuf,
    password: Option<String>,
    render_rx: Receiver<RenderRequest>,
    render_tx: Sender<RenderResult>,
    text_seg_tx: Sender<TextSegmentResult>,
    bt_rx: Receiver<BoundedTextRequest>,
    bt_tx: Sender<BoundedTextResult>,
    repaint: egui::Context,
    init_error: Arc<std::sync::Mutex<Option<String>>>,
) {
    // Keep a persistent Pdfium + document handle open for the lifetime of
    // this worker, avoiding repeated file open/close on every render.
    let pdfium = match super::renderer::pdfium() {
        Ok(p) => p,
        Err(err) => {
            log::error!("[PDF-RENDER] failed to init pdfium: {err}");
            if let Ok(mut slot) = init_error.lock() {
                *slot = Some(err);
            }
            repaint.request_repaint();
            return;
        }
    };
    let document = match pdfium.load_pdf_from_file(&path, password.as_deref()) {
        Ok(d) => d,
        Err(err) => {
            log::error!("[PDF-RENDER] failed to load document: {err}");
            if let Ok(mut slot) = init_error.lock() {
                *slot = Some(format!("LoadPdf: {err}"));
            }
            repaint.request_repaint();
            return;
        }
    };

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

    loop {
        // Drain high-priority bounded-text requests before blocking.
        drain_bounded_text(&document, &bt_rx, &bt_tx, &repaint, &ocr_cache);

        // Wait for either a render request or a bounded-text request.
        crossbeam_channel::select! {
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

                for (_, req) in latest {
                    // Prioritise bounded-text between page renders.
                    drain_bounded_text(&document, &bt_rx, &bt_tx, &repaint, &ocr_cache);

                    let page_idx = req.page_idx;

                    // Render page bitmap, also capturing natural page dimensions
                    // for OCR coordinate mapping.
                    let render_result = (|| -> Result<(super::renderer::RenderedPage, f32, f32), String> {
                        let page = document
                            .pages()
                            .get(page_idx as pdfium_render::prelude::PdfPageIndex)
                            .map_err(|e| e.to_string())?;

                        let page_w = page.width().value;
                        let page_h = page.height().value;

                        let bitmap = page
                            .render(
                                req.width as pdfium_render::prelude::Pixels,
                                req.height as pdfium_render::prelude::Pixels,
                                None,
                            )
                            .map_err(|e| format!("RenderPage: {e}"))?;

                        Ok((
                            super::renderer::RenderedPage {
                                width: bitmap.width() as u32,
                                height: bitmap.height() as u32,
                                pixels: bitmap.as_rgba_bytes(),
                            },
                            page_w,
                            page_h,
                        ))
                    })();

                    match render_result {
                        Ok((p, page_w, page_h)) => {
                            // Ensure segments are computed for this page.  On first
                            // visit: extract from Pdfium or run OCR.  On subsequent
                            // renders (zoom, scroll back into view): use the cache.
                            if !segment_cache.contains_key(&page_idx) {
                                let segments = match extract_text_segments(&document, page_idx) {
                                    Ok(segs) if !segs.is_empty() => segs,
                                    Ok(_) => {
                                        // No embedded text layer — try Windows OCR.
                                        run_ocr_canonical(
                                            &document,
                                            page_idx,
                                            &p.pixels,
                                            p.width,
                                            p.height,
                                            page_w,
                                            page_h,
                                            &mut ocr_cache,
                                        )
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "[PDF-RENDER] text segment extraction for page \
                                             {page_idx} failed: {e}"
                                        );
                                        vec![]
                                    }
                                };
                                if !segments.is_empty() {
                                    segment_cache.insert(page_idx, segments);
                                }
                            }

                            // Always re-send cached segments on every render so the
                            // UI-side page_text is refreshed after zoom or eviction.
                            if let Some(segments) = segment_cache.get(&page_idx) {
                                let _ = text_seg_tx.send(TextSegmentResult {
                                    page_idx,
                                    segments: segments.clone(),
                                });
                                repaint.request_repaint();
                            }

                            let _ = render_tx.send(RenderResult {
                                page_idx,
                                pixels: p.pixels,
                                width: p.width,
                                height: p.height,
                            });
                            repaint.request_repaint();
                        }
                        Err(e) => {
                            log::error!("[PDF-RENDER] page {page_idx} failed: {e}");
                        }
                    }
                }
            },
            recv(bt_rx) -> msg => {
                if let Ok(req) = msg {
                    handle_bounded_text(&document, &req, &bt_tx, &repaint, &ocr_cache);
                }
            },
        }
    }
}

// ── Text extraction helpers ──────────────────────────────────────────────────

fn drain_bounded_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    rx: &Receiver<BoundedTextRequest>,
    tx: &Sender<BoundedTextResult>,
    repaint: &egui::Context,
    ocr_cache: &HashMap<u32, Vec<OcrWord>>,
) {
    while let Ok(req) = rx.try_recv() {
        handle_bounded_text(document, &req, tx, repaint, ocr_cache);
    }
}

fn handle_bounded_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    req: &BoundedTextRequest,
    tx: &Sender<BoundedTextResult>,
    repaint: &egui::Context,
    ocr_cache: &HashMap<u32, Vec<OcrWord>>,
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
        ocr_cache
            .get(&req.page_idx)
            .map(|words| {
                words
                    .iter()
                    .filter(|w| w.bounds.overlaps(&req.bounds))
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    } else {
        pdfium_text
    };

    let _ = tx.send(BoundedTextResult {
        page_idx: req.page_idx,
        text,
    });
    repaint.request_repaint();
}

/// Canonical OCR side length (pixels).  Represents a good trade-off between
/// recognition quality and runtime: long side ~200 DPI for a Letter page.
const OCR_CANONICAL_SIDE: u32 = 1500;

/// Minimum display-bitmap long side accepted for OCR without a re-render.
/// Below this threshold a dedicated canonical bitmap is produced.
const OCR_MIN_DISPLAY_SIDE: u32 = 800;

/// Run Windows OCR for a page, selecting the bitmap source as follows:
/// - If the display bitmap (already rendered) meets `OCR_MIN_DISPLAY_SIDE`,
///   use it directly — no extra Pdfium call needed.
/// - Otherwise render a fresh canonical bitmap at `OCR_CANONICAL_SIDE` on the
///   longest side so OCR quality is independent of display zoom.
///
/// Results are stored in `ocr_cache` once and never replaced.
fn run_ocr_canonical(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    page_idx: u32,
    display_pixels: &[u8],
    display_w: u32,
    display_h: u32,
    page_w: f32,
    page_h: f32,
    ocr_cache: &mut HashMap<u32, Vec<OcrWord>>,
) -> Vec<PdfTextSegment> {
    let display_side = display_w.max(display_h);

    // Closure that converts OcrWords into display segments and updates the cache.
    let commit = |words: Vec<OcrWord>, cache: &mut HashMap<u32, Vec<OcrWord>>| {
        let segs = words
            .iter()
            .map(|w| PdfTextSegment { bounds: w.bounds })
            .collect::<Vec<_>>();
        cache.insert(page_idx, words);
        segs
    };

    if display_side >= OCR_MIN_DISPLAY_SIDE {
        // Reuse the display bitmap — no extra render.
        return match super::ocr::ocr_page_bitmap(
            display_pixels,
            display_w,
            display_h,
            page_w,
            page_h,
        ) {
            Some(words) => commit(words, ocr_cache),
            None => vec![],
        };
    }

    // Display bitmap is too small — render a dedicated canonical bitmap.
    let scale = OCR_CANONICAL_SIDE as f32 / page_w.max(page_h);
    let ocr_w = ((page_w * scale) as u32).max(1);
    let ocr_h = ((page_h * scale) as u32).max(1);

    let canonical = (|| -> Result<(Vec<u8>, u32, u32), String> {
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
            .map_err(|e| format!("OCR render: {e}"))?;
        Ok((bm.as_rgba_bytes(), bm.width() as u32, bm.height() as u32))
    })();

    match canonical {
        Ok((pixels, w, h)) => match super::ocr::ocr_page_bitmap(&pixels, w, h, page_w, page_h) {
            Some(words) => commit(words, ocr_cache),
            None => vec![],
        },
        Err(e) => {
            log::error!("[PDF-RENDER] canonical OCR render for page {page_idx} failed: {e}");
            vec![]
        }
    }
}

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
