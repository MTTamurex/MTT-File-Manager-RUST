//! Session state for the disk usage analyzer view.
//!
//! Owns a dedicated background worker that fetches the volume snapshot from
//! the search service and builds the immutable [`DiskAnalysisModel`] off the
//! UI thread. Results are matched by request id so stale fetches (drive
//! switch / view closed) are discarded.

use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// Drive facts the analyzer window needs. Snapshotted from `DriveState` when
/// the analyzer opens so the deferred viewport never borrows the main app.
#[derive(Clone)]
pub struct AnalyzerDriveSummary {
    pub letter: char,
    pub label: String,
    pub file_system: String,
    pub total_space: u64,
    pub free_space: u64,
}

pub struct DiskAnalysisRequest {
    pub drive_letter: char,
    pub request_id: u64,
}

pub enum DiskAnalysisMessage {
    ModelReady {
        request_id: u64,
        model: Arc<DiskAnalysisModel>,
        fetch_elapsed: Duration,
        /// Service-side volume index state ("ready", "scanning", ...).
        index_state: Option<String>,
    },
    Failed {
        request_id: u64,
        error: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskAnalysisPhase {
    Idle,
    Fetching,
    Ready,
    Failed,
}

pub struct DiskAnalysisState {
    /// Whether the analyzer view is currently displayed.
    pub active: bool,
    pub drive_letter: Option<char>,
    /// Drive snapshot (filled when the analyzer opens).
    pub drives: Vec<AnalyzerDriveSummary>,
    pub phase: DiskAnalysisPhase,
    pub model: Option<Arc<DiskAnalysisModel>>,
    pub error: Option<String>,
    pub fetch_elapsed: Option<Duration>,
    /// Service-side index state for the analyzed volume.
    pub index_state: Option<String>,
    /// Drill-down breadcrumb trail of node indices (last = current root).
    pub drill_stack: Vec<u32>,
    /// Currently hovered treemap node.
    pub hovered: Option<u32>,
    /// Set on close: the main loop compacts the heap on the next frame.
    pub reclaim_pending: bool,
    /// Native HWND of the analyzer viewport window (title-bar theming),
    /// stored as the raw pointer value to keep the state `Send`.
    #[cfg(target_os = "windows")]
    pub viewport_hwnd: Option<isize>,
    /// Last dark-mode flag applied to the viewport title bar.
    #[cfg(target_os = "windows")]
    pub viewport_title_bar_dark: Option<bool>,
    req_sender: Sender<DiskAnalysisRequest>,
    res_receiver: Receiver<DiskAnalysisMessage>,
    next_request_id: u64,
    active_request_id: u64,
}

impl DiskAnalysisState {
    pub fn new() -> Self {
        let (req_sender, req_receiver) = channel();
        let (res_sender, res_receiver) = channel();
        std::thread::Builder::new()
            .name("disk-analysis-worker".to_string())
            .spawn(move || worker_loop(req_receiver, res_sender))
            .expect("disk analysis worker thread should spawn");

        Self {
            active: false,
            drive_letter: None,
            drives: Vec::new(),
            phase: DiskAnalysisPhase::Idle,
            model: None,
            error: None,
            fetch_elapsed: None,
            index_state: None,
            drill_stack: Vec::new(),
            hovered: None,
            reclaim_pending: false,
            #[cfg(target_os = "windows")]
            viewport_hwnd: None,
            #[cfg(target_os = "windows")]
            viewport_title_bar_dark: None,
            req_sender,
            res_receiver,
            next_request_id: 0,
            active_request_id: 0,
        }
    }

    /// Start (or restart) a snapshot fetch for `drive_letter`. The previous
    /// model stays visible until the new one arrives.
    pub fn request(&mut self, drive_letter: char) {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }
        self.active_request_id = self.next_request_id;
        self.drive_letter = Some(drive_letter);
        self.phase = DiskAnalysisPhase::Fetching;
        self.error = None;
        self.hovered = None;
        if self
            .req_sender
            .send(DiskAnalysisRequest {
                drive_letter,
                request_id: self.active_request_id,
            })
            .is_err()
        {
            self.phase = DiskAnalysisPhase::Failed;
            self.error = Some("disk analysis worker unavailable".to_string());
        }
    }

    /// Drain worker messages. Returns true when the view must repaint data.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(message) = self.res_receiver.try_recv() {
            match message {
                DiskAnalysisMessage::ModelReady {
                    request_id,
                    model,
                    fetch_elapsed,
                    index_state,
                } => {
                    if request_id != self.active_request_id {
                        continue;
                    }
                    self.drill_stack = vec![model.root];
                    self.model = Some(model);
                    self.fetch_elapsed = Some(fetch_elapsed);
                    self.index_state = index_state;
                    self.phase = DiskAnalysisPhase::Ready;
                    self.hovered = None;
                    changed = true;
                }
                DiskAnalysisMessage::Failed { request_id, error } => {
                    if request_id != self.active_request_id {
                        continue;
                    }
                    self.phase = DiskAnalysisPhase::Failed;
                    self.error = Some(error);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Hide the view and drop the model; in-flight results are invalidated.
    /// Sets `reclaim_pending` so the main loop compacts the heap next frame.
    pub fn close(&mut self) {
        self.active = false;
        self.active_request_id = self.active_request_id.wrapping_add(1);
        self.phase = DiskAnalysisPhase::Idle;
        self.model = None;
        self.error = None;
        self.fetch_elapsed = None;
        self.index_state = None;
        self.drill_stack.clear();
        self.hovered = None;
        self.drives.clear();
        self.reclaim_pending = true;
    }
}

impl Default for DiskAnalysisState {
    fn default() -> Self {
        Self::new()
    }
}

fn worker_loop(
    req_receiver: Receiver<DiskAnalysisRequest>,
    res_sender: Sender<DiskAnalysisMessage>,
) {
    while let Ok(request) = req_receiver.recv() {
        let started = std::time::Instant::now();
        match crate::infrastructure::global_search::fetch_disk_analysis(request.drive_letter) {
            Ok(snapshot) => {
                let letter = snapshot.drive_letter.to_ascii_uppercase();
                let index_state = crate::infrastructure::global_search::get_status()
                    .ok()
                    .and_then(|status| {
                        status
                            .volumes
                            .iter()
                            .find(|v| v.drive_letter.to_ascii_uppercase() == letter)
                            .map(|v| v.state.clone())
                    });
                let model = Arc::new(DiskAnalysisModel::build(snapshot));
                let _ = res_sender.send(DiskAnalysisMessage::ModelReady {
                    request_id: request.request_id,
                    model,
                    fetch_elapsed: started.elapsed(),
                    index_state,
                });
            }
            Err(error) => {
                let _ = res_sender.send(DiskAnalysisMessage::Failed {
                    request_id: request.request_id,
                    error,
                });
            }
        }
    }
}
