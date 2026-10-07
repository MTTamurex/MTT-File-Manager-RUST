use std::path::PathBuf;

use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;

use super::renderer::{OpenedWorkbook, XlsxRenderer, XlsxSearchMatch, XlsxViewportTile};
use super::viewer_app::RenderKey;

pub(super) enum WorkerEvent {
    Opened(OpenedWorkbook),
    SheetSelected {
        sheet_index: usize,
        sheet_info: betteroffice_xlsx::SheetInfo,
    },
    Rendered {
        generation: u64,
        key: RenderKey,
        pixels: Vec<XlsxViewportTile>,
    },
    RenderFailed {
        generation: u64,
        key: RenderKey,
        error: String,
    },
    SearchCompleted {
        generation: u64,
        query: String,
        matches: Vec<XlsxSearchMatch>,
    },
    Failed(String),
}

enum WorkerRequest {
    SelectSheet(usize),
    Render { generation: u64, key: RenderKey },
    Search { generation: u64, query: String },
}

pub(super) struct XlsxRenderWorker {
    request_tx: Sender<WorkerRequest>,
    event_rx: Receiver<WorkerEvent>,
}

impl XlsxRenderWorker {
    pub fn spawn(path: PathBuf, repaint: egui::Context) -> Self {
        let (request_tx, request_rx) = bounded(8);
        let (event_tx, event_rx) = bounded(1);

        std::thread::Builder::new()
            .name("xlsx-render".to_owned())
            .spawn(move || worker_loop(path, request_rx, event_tx, repaint))
            .expect("spawn XLSX render worker");

        Self {
            request_tx,
            event_rx,
        }
    }

    pub fn request_sheet(&self, sheet_index: usize) -> bool {
        self.request_tx
            .try_send(WorkerRequest::SelectSheet(sheet_index))
            .is_ok()
    }

    pub fn request_render(&self, generation: u64, key: RenderKey) -> bool {
        self.request_tx
            .try_send(WorkerRequest::Render { generation, key })
            .is_ok()
    }

    pub fn request_search(&self, generation: u64, query: String) -> bool {
        self.request_tx
            .try_send(WorkerRequest::Search { generation, query })
            .is_ok()
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
    request_rx: Receiver<WorkerRequest>,
    event_tx: Sender<WorkerEvent>,
    repaint: egui::Context,
) {
    let (mut renderer, opened) = match XlsxRenderer::open(&path) {
        Ok(result) => result,
        Err(error) => {
            let _ = event_tx.send(WorkerEvent::Failed(error));
            repaint.request_repaint();
            return;
        }
    };

    if event_tx.send(WorkerEvent::Opened(opened)).is_err() {
        return;
    }
    repaint.request_repaint();

    while let Ok(request) = request_rx.recv() {
        let event = match request {
            WorkerRequest::SelectSheet(sheet_index) => match renderer.select_sheet(sheet_index) {
                Ok(sheet_info) => WorkerEvent::SheetSelected {
                    sheet_index,
                    sheet_info,
                },
                Err(error) => WorkerEvent::Failed(error),
            },
            WorkerRequest::Render { generation, key } => {
                let result = renderer.render_viewport(key.sheet_index, key.viewport);
                match result {
                    Ok(pixels) => WorkerEvent::Rendered {
                        generation,
                        key,
                        pixels,
                    },
                    Err(error) => WorkerEvent::RenderFailed {
                        generation,
                        key,
                        error,
                    },
                }
            }
            WorkerRequest::Search { generation, query } => WorkerEvent::SearchCompleted {
                generation,
                matches: renderer.search(&query),
                query,
            },
        };
        if event_tx.send(event).is_err() {
            break;
        }
        repaint.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerEvent, XlsxRenderWorker};
    use crossbeam_channel::bounded;

    #[test]
    fn reports_events_waiting_for_the_ui() {
        let (request_tx, _) = bounded(1);
        let (event_tx, event_rx) = bounded(1);
        let worker = XlsxRenderWorker {
            request_tx,
            event_rx,
        };
        assert!(!worker.has_pending_results());
        assert!(event_tx.send(WorkerEvent::Failed(String::new())).is_ok());
        assert!(worker.has_pending_results());
    }
}
