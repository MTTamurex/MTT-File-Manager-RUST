//! Session state for the disk usage analyzer view.
//!
//! Owns a dedicated background worker that fetches the volume snapshot from
//! the search service and builds the immutable [`DiskAnalysisModel`] off the
//! UI thread. Results are matched by request id so stale fetches (drive
//! switch / view closed) are discarded.

use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::sync::atomic::{AtomicU64, Ordering};
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
        drives: Vec<AnalyzerDriveSummary>,
        fetch_elapsed: Duration,
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
    pub drive_letter: Option<char>,
    /// Drive snapshot (filled when the analyzer opens).
    pub drives: Vec<AnalyzerDriveSummary>,
    pub phase: DiskAnalysisPhase,
    pub model: Option<Arc<DiskAnalysisModel>>,
    pub error: Option<String>,
    pub fetch_elapsed: Option<Duration>,
    /// Drill-down breadcrumb trail of node indices (last = current root).
    pub drill_stack: Vec<u32>,
    /// Currently hovered treemap node.
    pub hovered: Option<u32>,
    /// Right-click context menu target (node index); `None` when closed.
    pub context_menu: Option<u32>,
    pub(crate) treemap_cache: crate::ui::disk_analysis::treemap::LayoutCache,
    /// Icon cache for the analyzer window's own chrome (refresh button).
    pub svg_icons: crate::ui::svg_icons::SvgIconManager,
    /// Native HWND of the analyzer window (title-bar theming),
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
    active_generation: Arc<AtomicU64>,
}

impl DiskAnalysisState {
    pub fn new() -> Self {
        let (req_sender, req_receiver) = channel();
        let (res_sender, res_receiver) = channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = active_generation.clone();
        std::thread::Builder::new()
            .name("disk-analysis-worker".to_string())
            .spawn(move || worker_loop(req_receiver, res_sender, worker_generation))
            .expect("disk analysis worker thread should spawn");

        Self {
            drive_letter: None,
            drives: Vec::new(),
            phase: DiskAnalysisPhase::Idle,
            model: None,
            error: None,
            fetch_elapsed: None,
            drill_stack: Vec::new(),
            hovered: None,
            context_menu: None,
            treemap_cache: Default::default(),
            svg_icons: crate::ui::svg_icons::SvgIconManager::new(),
            #[cfg(target_os = "windows")]
            viewport_hwnd: None,
            #[cfg(target_os = "windows")]
            viewport_title_bar_dark: None,
            req_sender,
            res_receiver,
            next_request_id: 0,
            active_request_id: 0,
            active_generation,
        }
    }

    /// Start (or restart) a snapshot fetch for `drive_letter`. The previous
    /// model stays visible until the new one arrives.
    pub fn request(&mut self, drive_letter: char) {
        let drive_letter = drive_letter.to_ascii_uppercase();
        let switched_drive = self.drive_letter != Some(drive_letter);
        self.next_request_id = self.next_request_id.wrapping_add(1);
        if self.next_request_id == 0 {
            self.next_request_id = 1;
        }
        self.active_request_id = self.next_request_id;
        self.active_generation
            .store(self.active_request_id, Ordering::Release);
        self.drive_letter = Some(drive_letter);
        self.phase = DiskAnalysisPhase::Fetching;
        self.error = None;
        self.hovered = None;
        self.context_menu = None;
        if switched_drive {
            self.model = None;
            self.treemap_cache.clear();
            self.drill_stack.clear();
            self.fetch_elapsed = None;
        }
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
            changed |= self.apply_message(message);
        }
        changed
    }

    fn apply_message(&mut self, message: DiskAnalysisMessage) -> bool {
        match message {
            DiskAnalysisMessage::ModelReady {
                request_id,
                model,
                drives,
                fetch_elapsed,
            } => {
                if request_id != self.active_request_id
                    || Some(model.drive_letter) != self.drive_letter
                {
                    return false;
                }
                self.drill_stack = vec![model.root];
                self.model = Some(model);
                self.treemap_cache.clear();
                self.drives = drives;
                self.fetch_elapsed = Some(fetch_elapsed);
                self.phase = DiskAnalysisPhase::Ready;
                self.hovered = None;
                self.context_menu = None;
                true
            }
            DiskAnalysisMessage::Failed { request_id, error } => {
                if request_id != self.active_request_id {
                    return false;
                }
                self.phase = DiskAnalysisPhase::Failed;
                self.error = Some(error);
                self.model = None;
                self.treemap_cache.clear();
                self.drill_stack.clear();
                self.fetch_elapsed = None;
                true
            }
        }
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
    active_generation: Arc<AtomicU64>,
) {
    'requests: while let Ok(mut request) = req_receiver.recv() {
        while let Ok(newer) = req_receiver.try_recv() {
            request = newer;
        }
        let request_id = request.request_id;
        let is_cancelled = || active_generation.load(Ordering::Acquire) != request_id;
        if is_cancelled() {
            continue;
        }
        let started = std::time::Instant::now();
        let snapshot = loop {
            match crate::infrastructure::global_search::fetch_disk_analysis_cancellable(
                request.drive_letter,
                is_cancelled,
            ) {
                Ok(Some(snapshot)) => break snapshot,
                Ok(None) => continue 'requests,
                Err(error)
                    if !is_cancelled()
                        && error == mtt_search_protocol::DISK_ANALYSIS_BUSY_ERROR
                        && started.elapsed() < Duration::from_secs(140) =>
                {
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(error)
                    if !is_cancelled()
                        && error == mtt_search_protocol::DISK_ANALYSIS_REFRESHING_ERROR
                        && started.elapsed() < Duration::from_secs(10) =>
                {
                    std::thread::sleep(Duration::from_millis(250));
                }
                Err(error) => {
                    if !is_cancelled() {
                        let _ = res_sender.send(DiskAnalysisMessage::Failed { request_id, error });
                    }
                    continue 'requests;
                }
            }
        };
        let Some(model) = DiskAnalysisModel::build_cancellable(snapshot, is_cancelled) else {
            continue;
        };
        if is_cancelled() {
            continue;
        }
        let _ = res_sender.send(DiskAnalysisMessage::ModelReady {
            request_id,
            model: Arc::new(model),
            drives: collect_drive_summaries(),
            fetch_elapsed: started.elapsed(),
        });
    }
}

pub fn collect_drive_summaries() -> Vec<AnalyzerDriveSummary> {
    let (disks, _unavailable_label_roots) =
        crate::infrastructure::windows::get_all_drives_with_label_status();
    let mut drives = Vec::new();
    for (path, label) in disks {
        let Some(letter) = path.chars().next().filter(|c| c.is_ascii_alphabetic()) else {
            continue;
        };
        let vol = crate::infrastructure::windows::get_volume_info(&path);
        drives.push(AnalyzerDriveSummary {
            letter: letter.to_ascii_uppercase(),
            label,
            file_system: vol.file_system,
            total_space: vol.total_space,
            free_space: vol.free_space,
        });
    }
    drives
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

    fn model(letter: char) -> Arc<DiskAnalysisModel> {
        Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
            drive_letter: letter,
            records: vec![DiskAnalysisRecord {
                frn: 5,
                parent_frn: 5,
                name: String::new(),
                size: 0,
                allocated_size: 0,
                is_dir: true,
                is_reparse: false,
            }],
        }))
    }

    #[test]
    fn switching_drive_clears_previous_model() {
        let mut state = DiskAnalysisState::new();
        state.drive_letter = Some('C');
        state.model = Some(model('C'));
        state.drill_stack = vec![1];

        state.request('D');

        assert_eq!(state.drive_letter, Some('D'));
        assert!(state.model.is_none());
        assert!(state.drill_stack.is_empty());
    }

    #[test]
    fn same_drive_refresh_keeps_model_until_result() {
        let mut state = DiskAnalysisState::new();
        state.drive_letter = Some('C');
        state.model = Some(model('C'));

        state.request('C');

        assert!(state.model.is_some());
        assert_eq!(state.phase, DiskAnalysisPhase::Fetching);
    }

    #[test]
    fn active_failure_removes_stale_model() {
        let mut state = DiskAnalysisState::new();
        state.drive_letter = Some('C');
        state.model = Some(model('C'));
        state.active_request_id = 7;

        assert!(state.apply_message(DiskAnalysisMessage::Failed {
            request_id: 7,
            error: "failed".to_string(),
        }));
        assert!(state.model.is_none());
        assert_eq!(state.phase, DiskAnalysisPhase::Failed);
    }

    #[test]
    fn wrong_drive_result_is_ignored() {
        let mut state = DiskAnalysisState::new();
        state.drive_letter = Some('D');
        state.active_request_id = 7;

        assert!(!state.apply_message(DiskAnalysisMessage::ModelReady {
            request_id: 7,
            model: model('C'),
            drives: Vec::new(),
            fetch_elapsed: Duration::ZERO,
        }));
        assert!(state.model.is_none());
    }
}
