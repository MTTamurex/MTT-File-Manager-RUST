//! Background PDF page rendering worker.
//!
//! Owns the [`PdfRenderer`] and processes render requests on a dedicated
//! thread so the UI stays fluid.  Results are sent back via channel and
//! the egui context is poked to trigger a repaint.

use super::renderer::PdfRenderer;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;

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
}

impl RenderWorker {
    /// Spawn the worker thread.  Takes ownership of the renderer.
    pub fn spawn(renderer: PdfRenderer, repaint: egui::Context) -> Self {
        let (req_tx, req_rx) = crossbeam_channel::unbounded();
        let (res_tx, res_rx) = crossbeam_channel::unbounded();

        std::thread::Builder::new()
            .name("pdf-render".into())
            .spawn(move || worker_loop(renderer, req_rx, res_tx, repaint))
            .expect("spawn pdf-render thread");

        Self {
            tx: req_tx,
            rx: res_rx,
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
}

fn worker_loop(
    renderer: PdfRenderer,
    rx: Receiver<RenderRequest>,
    tx: Sender<RenderResult>,
    repaint: egui::Context,
) {
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
            match renderer.render_page(req.page_idx, req.width, req.height) {
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
