//! Background PDF page rendering worker.
//!
//! Owns the [`PdfRenderer`] and processes render requests on a dedicated
//! thread so the UI stays fluid.  Results are sent back via channel and
//! the egui context is poked to trigger a repaint.

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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

pub(super) struct RenderWorker {
    tx: Sender<RenderRequest>,
    rx: Receiver<RenderResult>,
    /// Set by the worker thread if PDF initialisation fails.
    init_error: Arc<std::sync::Mutex<Option<String>>>,
}

impl RenderWorker {
    /// Spawn the worker thread.
    pub fn spawn(path: PathBuf, repaint: egui::Context) -> Self {
        let (req_tx, req_rx) = crossbeam_channel::bounded(32);
        let (res_tx, res_rx) = crossbeam_channel::bounded(64);
        let init_error = Arc::new(std::sync::Mutex::new(None));
        let init_error_w = Arc::clone(&init_error);

        std::thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || worker_loop(path, req_rx, res_tx, repaint, init_error_w))
            .expect("spawn pdf-render thread");

        Self {
            tx: req_tx,
            rx: res_rx,
            init_error,
        }
    }

    /// Submit a non-blocking render request.
    pub fn request(&self, req: RenderRequest) {
        let _ = self.tx.send(req);
    }

    /// Drain all completed results from the channel.
    pub fn drain_results(&self) -> Vec<RenderResult> {
        let mut out = Vec::new();
        while let Ok(r) = self.rx.try_recv() {
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
    rx: Receiver<RenderRequest>,
    tx: Sender<RenderResult>,
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

    loop {
        // Block until first request
        let first = match rx.recv() {
            Ok(r) => r,
            Err(_) => return, // channel closed — exit
        };

        // Drain + dedup: keep only the latest request per page
        let mut latest: HashMap<u32, RenderRequest> = HashMap::new();
        latest.insert(first.page_idx, first);
        while let Ok(r) = rx.try_recv() {
            latest.insert(r.page_idx, r);
        }

        for (_, req) in latest {
            // Use the cached document handle instead of reopening.
            let render_result = (|| -> Result<super::renderer::RenderedPage, String> {
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
                    .map_err(|e| format!("RenderPage: {e}"))?;

                Ok(super::renderer::RenderedPage {
                    width: bitmap.width() as u32,
                    height: bitmap.height() as u32,
                    pixels: bitmap.as_rgba_bytes(),
                })
            })();

            match render_result {
                Ok(p) => {
                    let _ = tx.send(RenderResult {
                        page_idx: req.page_idx,
                        pixels: p.pixels,
                        width: p.width,
                        height: p.height,
                    });
                    repaint.request_repaint();
                }
                Err(e) => {
                    log::error!("[PDF-RENDER] page {} failed: {e}", req.page_idx);
                }
            }
        }
    }
}
