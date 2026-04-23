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
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

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
    pub fn spawn(path: PathBuf, repaint: egui::Context) -> Self {
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
    let document = match pdfium.load_pdf_from_file(&path, None) {
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

    let mut text_extracted: HashSet<u32> = HashSet::new();

    loop {
        // Drain high-priority bounded-text requests before blocking.
        drain_bounded_text(&document, &bt_rx, &bt_tx, &repaint);

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
                    drain_bounded_text(&document, &bt_rx, &bt_tx, &repaint);

                    let page_idx = req.page_idx;

                    // Render page bitmap.
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
                            });
                            repaint.request_repaint();
                        }
                        Err(e) => {
                            log::error!("[PDF-RENDER] page {} failed: {e}", page_idx);
                        }
                    }

                    // Eagerly extract text segments on first render of each page.
                    if !text_extracted.contains(&page_idx) {
                        text_extracted.insert(page_idx);
                        match extract_text_segments(&document, page_idx) {
                            Ok(segments) => {
                                let _ = text_seg_tx.send(TextSegmentResult {
                                    page_idx,
                                    segments,
                                });
                                repaint.request_repaint();
                            }
                            Err(e) => {
                                log::error!(
                                    "[PDF-RENDER] text segment extraction for page {} failed: {e}",
                                    page_idx
                                );
                            }
                        }
                    }
                }
            },
            recv(bt_rx) -> msg => {
                if let Ok(req) = msg {
                    handle_bounded_text(&document, &req, &bt_tx, &repaint);
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
) {
    while let Ok(req) = rx.try_recv() {
        handle_bounded_text(document, &req, tx, repaint);
    }
}

fn handle_bounded_text(
    document: &pdfium_render::prelude::PdfDocument<'_>,
    req: &BoundedTextRequest,
    tx: &Sender<BoundedTextResult>,
    repaint: &egui::Context,
) {
    let text = extract_bounded_text(document, req.page_idx, req.bounds).unwrap_or_else(|e| {
        log::warn!(
            "[PDF-RENDER] bounded text for page {} failed: {e}",
            req.page_idx
        );
        String::new()
    });
    let _ = tx.send(BoundedTextResult {
        page_idx: req.page_idx,
        text,
    });
    repaint.request_repaint();
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
    Ok(text.inside_rect(pdfium_render::prelude::PdfRect::new_from_values(
        bounds.bottom,
        bounds.left,
        bounds.top,
        bounds.right,
    )))
}
