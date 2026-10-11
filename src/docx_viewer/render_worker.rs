use std::path::PathBuf;

use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;

use super::renderer::{DocxPagePixels, DocxRenderer};
use super::search::{DocxSearchIndex, SearchBounds, SearchRequest, SearchResult};

pub(super) enum WorkerEvent {
    Opened {
        page_sizes: Vec<(f32, f32)>,
    },
    Rendered(DocxPagePixels),
    TextSelected {
        page_index: usize,
        generation: u64,
        text: String,
    },
    PageFailed {
        page_index: usize,
        error: String,
    },
    Failed(String),
}

enum RenderRequest {
    Page(usize),
    SelectText {
        page_index: usize,
        bounds: SearchBounds,
        generation: u64,
    },
    TrimCaches,
}

pub(super) struct DocxRenderWorker {
    request_tx: Sender<RenderRequest>,
    event_rx: Receiver<WorkerEvent>,
    search_request_tx: Sender<SearchRequest>,
    search_result_rx: Receiver<SearchResult>,
}

impl DocxRenderWorker {
    pub fn spawn(path: PathBuf, repaint: egui::Context) -> Self {
        let (request_tx, request_rx) = bounded(24);
        let (search_request_tx, search_request_rx) = crossbeam_channel::unbounded();
        let (search_result_tx, search_result_rx) = bounded(4);
        // Rendered pages can each hold tens of MiB. Keep one result waiting
        // for the UI while the worker may be finishing the next page.
        let (event_tx, event_rx) = bounded(1);

        std::thread::Builder::new()
            .name("docx-render".to_owned())
            .spawn(move || {
                worker_loop(
                    path,
                    request_rx,
                    event_tx,
                    search_request_rx,
                    search_result_tx,
                    repaint,
                )
            })
            .expect("spawn DOCX render worker");

        Self {
            request_tx,
            event_rx,
            search_request_tx,
            search_result_rx,
        }
    }

    pub fn request_page(&self, page_index: usize) -> bool {
        self.request_tx
            .try_send(RenderRequest::Page(page_index))
            .is_ok()
    }

    pub fn request_text_selection(
        &self,
        page_index: usize,
        bounds: SearchBounds,
        generation: u64,
    ) -> bool {
        self.request_tx
            .try_send(RenderRequest::SelectText {
                page_index,
                bounds,
                generation,
            })
            .is_ok()
    }

    pub fn request_cache_trim(&self) -> bool {
        self.request_tx.try_send(RenderRequest::TrimCaches).is_ok()
    }

    pub fn request_search(&self, request: SearchRequest) {
        let _ = self.search_request_tx.send(request);
    }

    pub fn drain_search_results(&self) -> Vec<SearchResult> {
        let mut results = Vec::new();
        while let Ok(result) = self.search_result_rx.try_recv() {
            results.push(result);
        }
        results
    }

    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn has_pending_results(&self) -> bool {
        !self.event_rx.is_empty() || !self.search_result_rx.is_empty()
    }
}

fn worker_loop(
    path: PathBuf,
    request_rx: Receiver<RenderRequest>,
    event_tx: Sender<WorkerEvent>,
    search_request_rx: Receiver<SearchRequest>,
    search_result_tx: Sender<SearchResult>,
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

    let search_index = DocxSearchIndex::new(renderer.search_display_list());
    let search_repaint = repaint.clone();
    std::thread::Builder::new()
        .name("docx-search".to_owned())
        .spawn(move || {
            super::search::run_search_worker(
                search_index,
                search_request_rx,
                search_result_tx,
                search_repaint,
            )
        })
        .expect("spawn DOCX search worker");

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
            RenderRequest::SelectText {
                page_index,
                bounds,
                generation,
            } => {
                if event_tx
                    .send(WorkerEvent::TextSelected {
                        page_index,
                        generation,
                        text: renderer.selected_text(page_index, bounds),
                    })
                    .is_err()
                {
                    break;
                }
                repaint.request_repaint();
            }
        }
    }
}
