use std::path::PathBuf;

use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;

use super::renderer::{DocxPagePixels, DocxRenderer};

pub(super) enum WorkerEvent {
    Opened { page_sizes: Vec<(f32, f32)> },
    Rendered(DocxPagePixels),
    PageFailed { page_index: usize, error: String },
    Failed(String),
}

enum RenderRequest {
    Page(usize),
    TrimCaches,
}

pub(super) struct DocxRenderWorker {
    request_tx: Sender<RenderRequest>,
    event_rx: Receiver<WorkerEvent>,
}

impl DocxRenderWorker {
    pub fn spawn(path: PathBuf, repaint: egui::Context) -> Self {
        let (request_tx, request_rx) = bounded(24);
        // Rendered pages can each hold tens of MiB. Keep one result waiting
        // for the UI while the worker may be finishing the next page.
        let (event_tx, event_rx) = bounded(1);

        std::thread::Builder::new()
            .name("docx-render".to_owned())
            .spawn(move || worker_loop(path, request_rx, event_tx, repaint))
            .expect("spawn DOCX render worker");

        Self {
            request_tx,
            event_rx,
        }
    }

    pub fn request_page(&self, page_index: usize) -> bool {
        self.request_tx
            .try_send(RenderRequest::Page(page_index))
            .is_ok()
    }

    pub fn request_cache_trim(&self) -> bool {
        self.request_tx.try_send(RenderRequest::TrimCaches).is_ok()
    }

    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn has_pending_results(&self) -> bool {
        !self.event_rx.is_empty()
    }
}

fn worker_loop(
    path: PathBuf,
    request_rx: Receiver<RenderRequest>,
    event_tx: Sender<WorkerEvent>,
    repaint: egui::Context,
) {
    let mut renderer = match DocxRenderer::open(&path) {
        Ok(renderer) => renderer,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Failed(error));
            repaint.request_repaint();
            return;
        }
    };

    if event_tx
        .send(WorkerEvent::Opened {
            page_sizes: renderer.page_sizes().to_vec(),
        })
        .is_err()
    {
        return;
    }
    repaint.request_repaint();

    while let Ok(request) = request_rx.recv() {
        match request {
            RenderRequest::TrimCaches => renderer.trim_caches(),
            RenderRequest::Page(page_index) => {
                let event = match renderer.render_page(page_index) {
                    Ok(page) => WorkerEvent::Rendered(page),
                    Err(error) => WorkerEvent::PageFailed { page_index, error },
                };
                if event_tx.send(event).is_err() {
                    break;
                }
                repaint.request_repaint();
            }
        }
    }
}
