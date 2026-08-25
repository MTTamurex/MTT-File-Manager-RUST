//! Session state for the disk usage analyzer view.
//!
//! Owns dedicated background workers:
//! - the fetch worker that pulls the volume snapshot from the search service
//!   and builds the immutable [`DiskAnalysisModel`] off the UI thread;
//! - a fast query worker running filter rollups, top-K lists, subtree search
//!   and efficiency scans (all pure walks over the model).
//!
//! Every job carries a generation id so stale results (drive switch / model
//! change / newer edit) are discarded on arrival instead of being applied.

use crate::app::disk_analysis_model::DiskAnalysisModel;
use crate::app::disk_analysis_query::{
    ActiveWeights, DiskAnalysisFilter, EfficiencyResult, FilterSizeBase, SizeMetric,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the UI waits after the last keystroke before scheduling a new
/// filter/search computation.
pub const QUERY_DEBOUNCE: Duration = Duration::from_millis(250);

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

/// Jobs accepted by the query worker. All of them snapshot the inputs they
/// need (`Arc` model clone) so the UI never shares mutable state.
pub enum QueryRequest {
    /// Filtered per-node weights over `root`.
    Weights {
        job: u64,
        cancel: Arc<AtomicU64>,
        model: Arc<DiskAnalysisModel>,
        root: u32,
        filter: DiskAnalysisFilter,
        metric: SizeMetric,
    },
    /// Top-K heaviest descendants of `root`.
    Largest {
        job: u64,
        cancel: Arc<AtomicU64>,
        model: Arc<DiskAnalysisModel>,
        root: u32,
        metric: SizeMetric,
        weights: Option<Arc<Vec<u64>>>,
    },
    /// Efficiency scan (logical vs allocated) over `root`.
    Efficiency {
        job: u64,
        cancel: Arc<AtomicU64>,
        model: Arc<DiskAnalysisModel>,
        root: u32,
        matches: Option<Arc<Vec<bool>>>,
    },
    /// Case-insensitive name/path search under `root`.
    Search {
        job: u64,
        cancel: Arc<AtomicU64>,
        model: Arc<DiskAnalysisModel>,
        root: u32,
        query: String,
        matches: Option<Arc<Vec<bool>>>,
    },
}

pub enum QueryMessage {
    Weights {
        job: u64,
        root: u32,
        metric: SizeMetric,
        weights: Arc<Vec<u64>>,
        matches: Arc<Vec<bool>>,
    },
    Largest {
        job: u64,
        rows: Arc<Vec<u32>>,
    },
    Efficiency {
        job: u64,
        result: Arc<EfficiencyResult>,
    },
    Search {
        job: u64,
        hits: Arc<Vec<u32>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskAnalysisPhase {
    Idle,
    Fetching,
    Ready,
    Failed,
}

/// Tabs of the bottom results panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResultsTab {
    #[default]
    Largest,
    Efficiency,
    Duplicates,
}

/// Sortable columns of the Largest table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LargestColumn {
    Name,
    Path,
    Logical,
    Allocated,
    Difference,
    Files,
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
    /// Persistently selected node (click in treemap or result rows); kept
    /// separate from hover and from the context-menu target so highlights
    /// survive pointer movement.
    pub selected: Option<u32>,
    /// Quantity driving the treemap, charts and tables.
    pub metric: SizeMetric,

    // ---- Filters -------------------------------------------------------
    /// Normalized active filter (rebuilt from the draft fields below).
    pub(crate) filter: DiskAnalysisFilter,
    /// Raw extension text field ("pdf, .PNG"), normalized on use.
    pub filter_extensions_text: String,
    /// Raw min/max size text fields (parsed as u64 bytes; empty = unset).
    pub filter_min_size_text: String,
    pub filter_max_size_text: String,
    pub filter_categories_mask: u8,
    pub filter_size_base: FilterSizeBase,
    pub filters_visible: bool,
    /// Bumped whenever any filter input changes; results carry this id.
    pub(crate) filter_gen: u64,
    /// Debounce deadline for the pending filter recompute, if any.
    filter_pending_at: Option<Instant>,
    filter_scheduled_sig: Option<(u64, u32, SizeMetric)>,
    filter_job: u64,
    weights_pending: bool,
    /// Latest computed filtered weights (id == matching `filter_gen`+root).
    pub(crate) active_weights: Option<ActiveWeights>,
    /// Signature of the largest-items job currently queued/attached.
    largest_sig: Option<(usize, u32, u64, SizeMetric)>,
    largest_job: u64,
    largest_pending: bool,
    /// Signature of the efficiency job currently queued/attached.
    efficiency_sig: Option<(usize, u32, u64)>,
    efficiency_job: u64,
    efficiency_pending: bool,
    /// Signature of the search job currently queued (model, root, weights,
    /// normalized query).
    search_sig: Option<(usize, u32, u64, String)>,

    // ---- Bottom panel --------------------------------------------------
    pub results_tab: ResultsTab,
    pub results_height: f32,
    pub results_collapsed: bool,
    pub largest_rows: Arc<Vec<u32>>,
    pub largest_sort_column: LargestColumn,
    pub largest_sort_asc: bool,
    /// One-shot scroll request: scroll the Largest table to this row index.
    pub largest_scroll_to_row: Option<usize>,
    pub efficiency_result: Option<Arc<EfficiencyResult>>,

    // ---- Search --------------------------------------------------------
    pub search_text: String,
    pub(crate) search_pending_at: Option<Instant>,
    search_job: u64,
    search_worker_pending: bool,
    pub search_results: Arc<Vec<u32>>,
    pub search_selected: usize,
    /// Whether the search popup list is visible (has query + focus history).
    pub search_open: bool,

    // ---- Duplicates ----------------------------------------------------
    pub duplicates: crate::app::disk_analysis_duplicates::DuplicateSession,

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
    query_sender: Sender<QueryRequest>,
    query_receiver: Receiver<QueryMessage>,
    next_request_id: u64,
    active_request_id: u64,
    active_generation: Arc<AtomicU64>,
    next_job_id: u64,
    query_context: Option<(usize, u32, SizeMetric)>,
    weights_generation: Arc<AtomicU64>,
    largest_generation: Arc<AtomicU64>,
    efficiency_generation: Arc<AtomicU64>,
    search_generation: Arc<AtomicU64>,
}

impl DiskAnalysisState {
    pub fn new() -> Self {
        let (req_sender, req_receiver) = channel();
        let (res_sender, res_receiver) = channel();
        let (query_sender, query_receiver) = channel();
        let (query_res_sender, query_res_receiver) = channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let weights_generation = Arc::new(AtomicU64::new(0));
        let largest_generation = Arc::new(AtomicU64::new(0));
        let efficiency_generation = Arc::new(AtomicU64::new(0));
        let search_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = active_generation.clone();
        std::thread::Builder::new()
            .name("disk-analysis-worker".to_string())
            .spawn(move || worker_loop(req_receiver, res_sender, worker_generation))
            .expect("disk analysis worker thread should spawn");
        std::thread::Builder::new()
            .name("disk-analysis-query".to_string())
            .spawn(move || query_worker_loop(query_receiver, query_res_sender))
            .expect("disk analysis query worker thread should spawn");

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
            selected: None,
            metric: SizeMetric::default(),
            filter: DiskAnalysisFilter::default(),
            filter_extensions_text: String::new(),
            filter_min_size_text: String::new(),
            filter_max_size_text: String::new(),
            filter_categories_mask: DiskAnalysisFilter::all_categories_mask(),
            filter_size_base: FilterSizeBase::default(),
            filters_visible: false,
            filter_gen: 0,
            filter_pending_at: None,
            filter_scheduled_sig: None,
            filter_job: 0,
            weights_pending: false,
            active_weights: None,
            largest_sig: None,
            largest_job: 0,
            largest_pending: false,
            efficiency_sig: None,
            efficiency_job: 0,
            efficiency_pending: false,
            search_sig: None,
            results_tab: ResultsTab::default(),
            results_height: 240.0,
            results_collapsed: false,
            largest_rows: Arc::new(Vec::new()),
            largest_sort_column: LargestColumn::Allocated,
            largest_sort_asc: false,
            largest_scroll_to_row: None,
            efficiency_result: None,
            search_text: String::new(),
            search_pending_at: None,
            search_job: 0,
            search_worker_pending: false,
            search_results: Arc::new(Vec::new()),
            search_selected: 0,
            search_open: false,
            duplicates: crate::app::disk_analysis_duplicates::DuplicateSession::new(),
            treemap_cache: Default::default(),
            svg_icons: crate::ui::svg_icons::SvgIconManager::new(),
            #[cfg(target_os = "windows")]
            viewport_hwnd: None,
            #[cfg(target_os = "windows")]
            viewport_title_bar_dark: None,
            req_sender,
            res_receiver,
            query_sender,
            query_receiver: query_res_receiver,
            next_request_id: 0,
            active_request_id: 0,
            active_generation,
            next_job_id: 1,
            query_context: None,
            weights_generation,
            largest_generation,
            efficiency_generation,
            search_generation,
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
        self.reset_view_state(switched_drive);
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

    /// Invalidate transient view/consultation state. `drop_model` also clears
    /// the model itself plus everything derived from it (drive switch).
    pub(crate) fn reset_view_state(&mut self, drop_model: bool) {
        self.invalidate_query_jobs();
        self.hovered = None;
        self.context_menu = None;
        self.selected = None;
        self.search_open = false;
        self.search_pending_at = None;
        self.search_results = Arc::new(Vec::new());
        self.search_selected = 0;
        self.largest_rows = Arc::new(Vec::new());
        self.efficiency_result = None;
        self.largest_scroll_to_row = None;
        self.duplicates.invalidate();
        self.active_weights = None;
        self.filter_pending_at = None;
        self.filter_scheduled_sig = None;
        self.largest_sig = None;
        self.efficiency_sig = None;
        self.search_sig = None;
        self.query_context = None;
        if drop_model {
            self.model = None;
            self.treemap_cache.clear();
            self.drill_stack.clear();
            self.fetch_elapsed = None;
        }
    }

    /// Rebuild the structured filter from the raw text fields.
    pub fn refresh_filter_from_inputs(&mut self) {
        let extensions = DiskAnalysisFilter::parse_extensions(&self.filter_extensions_text);
        let parse_size = |text: &str| text.trim().parse::<u64>().ok();
        let mut next = DiskAnalysisFilter {
            categories_mask: self.filter_categories_mask,
            extensions,
            min_size: parse_size(&self.filter_min_size_text),
            max_size: parse_size(&self.filter_max_size_text),
            size_base: self.filter_size_base,
        };
        if !next.is_active() {
            next = DiskAnalysisFilter::default();
        }
        self.filter = next;
    }

    /// Called by the toolbar whenever a filter input changes.
    pub fn mark_filter_changed(&mut self) {
        self.refresh_filter_from_inputs();
        self.filter_gen = self.filter_gen.wrapping_add(1);
        self.filter_pending_at = Some(Instant::now());
        self.active_weights = None;
        self.invalidate_query_jobs();
        self.clear_query_results();
    }

    /// Called whenever the search draft changes. Cancels an in-flight search
    /// immediately; the replacement is scheduled after the debounce.
    pub fn mark_search_changed(&mut self) {
        self.search_pending_at = Some(Instant::now());
        self.search_sig = None;
        self.search_job = self.alloc_job();
        self.search_generation
            .store(self.search_job, Ordering::Release);
        self.search_worker_pending = false;
        self.search_results = Arc::new(Vec::new());
        self.search_selected = 0;
    }

    /// Current subtree root (last drilled node or the volume root).
    pub fn current_root(&self) -> Option<u32> {
        self.model
            .as_ref()
            .map(|m| *self.drill_stack.last().unwrap_or(&m.root))
    }

    /// Navigate the drill stack to `idx`, keeping the full ancestor chain.
    /// Returns true when navigation happened.
    pub fn navigate_to(&mut self, idx: u32) -> bool {
        let Some(model) = self.model.clone() else {
            return false;
        };
        let Some(node) = model.nodes.get(idx as usize) else {
            return false;
        };
        if !node.is_dir || node.is_reparse {
            return false;
        }
        self.drill_stack = model.chain_to(idx);
        self.hovered = None;
        true
    }

    /// Select a file: drill into its parent folder and highlight it.
    pub fn reveal_file(&mut self, idx: u32) -> bool {
        let Some(model) = self.model.clone() else {
            return false;
        };
        let Some(node) = model.nodes.get(idx as usize) else {
            return false;
        };
        if node.is_dir {
            return false;
        }
        let parent = node.parent;
        if parent != idx {
            self.drill_stack = model.chain_to(parent);
        }
        self.selected = Some(idx);
        self.hovered = None;
        true
    }

    /// Schedule follow-up query jobs after any state change (navigation,
    /// filter edits, new model, metric switch). Cheap to call every frame:
    /// work is only queued when a signature changes.
    pub fn sync_query_jobs(&mut self) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let Some(root) = self.current_root() else {
            return;
        };

        let context = (Arc::as_ptr(&model) as usize, root, self.metric);
        if self.query_context != Some(context) {
            if self.query_context.is_some_and(|(model_ptr, old_root, _)| {
                model_ptr != context.0 || old_root != context.1
            }) {
                self.duplicates.invalidate();
            }
            self.query_context = Some(context);
            self.invalidate_dependent_query_jobs();
            self.clear_query_results();
        }

        let stale_weights = self
            .active_weights
            .as_ref()
            .is_some_and(|w| w.root != root || w.metric != self.metric);
        if stale_weights {
            self.active_weights = None;
            self.invalidate_dependent_query_jobs();
            self.clear_query_results();
        }

        // 1. Filtered weights (debounced; skipped entirely when inactive).
        if self.filter.is_active() {
            let sig = (self.filter_gen, root, self.metric);
            let due = self
                .filter_pending_at
                .map(|at| at.elapsed() >= QUERY_DEBOUNCE)
                .unwrap_or(true);
            if self.filter_scheduled_sig != Some(sig) && due {
                self.filter_job = self.alloc_job();
                self.weights_generation
                    .store(self.filter_job, Ordering::Release);
                self.filter_scheduled_sig = Some(sig);
                self.filter_pending_at = None;
                self.weights_pending = self
                    .query_sender
                    .send(QueryRequest::Weights {
                        job: self.filter_job,
                        cancel: self.weights_generation.clone(),
                        model: model.clone(),
                        root,
                        filter: self.filter.clone(),
                        metric: self.metric,
                    })
                    .is_ok();
            }
        } else {
            self.active_weights = None;
            self.filter_scheduled_sig = None;
            self.filter_pending_at = None;
            self.weights_pending = false;
        }
        let weights_valid = self
            .active_weights
            .as_ref()
            .map(|w| w.root == root && w.metric == self.metric && w.id == self.filter_job)
            .unwrap_or(!self.filter.is_active());
        if self.filter.is_active() && !weights_valid {
            return;
        }
        let weights_id = self.active_weights.as_ref().map(|w| w.id).unwrap_or(0);
        let weights_for_jobs = if weights_valid {
            self.active_weights.as_ref().map(|w| w.weights.clone())
        } else {
            None
        };

        // 2. Largest top-K.
        let largest_sig = (Arc::as_ptr(&model) as usize, root, weights_id, self.metric);
        if self.largest_sig != Some(largest_sig) {
            self.largest_sig = Some(largest_sig);
            let job = self.alloc_job();
            self.largest_job = job;
            self.largest_generation.store(job, Ordering::Release);
            self.largest_rows = Arc::new(Vec::new());
            self.largest_pending = self
                .query_sender
                .send(QueryRequest::Largest {
                    job,
                    cancel: self.largest_generation.clone(),
                    model: model.clone(),
                    root,
                    metric: self.metric,
                    weights: weights_for_jobs.clone(),
                })
                .is_ok();
        }

        // 3. Efficiency scan.
        let eff_sig = (Arc::as_ptr(&model) as usize, root, weights_id);
        if self.efficiency_sig != Some(eff_sig) {
            self.efficiency_sig = Some(eff_sig);
            let job = self.alloc_job();
            self.efficiency_job = job;
            self.efficiency_generation.store(job, Ordering::Release);
            self.efficiency_result = None;
            self.efficiency_pending = self
                .query_sender
                .send(QueryRequest::Efficiency {
                    job,
                    cancel: self.efficiency_generation.clone(),
                    model: model.clone(),
                    root,
                    matches: self.active_weights.as_ref().map(|w| w.matches.clone()),
                })
                .is_ok();
        }

        // 4. Search (debounced on its own).
        let trimmed = self.search_text.trim().to_string();
        if trimmed.is_empty() {
            if !self.search_results.is_empty() || self.search_open {
                self.search_results = Arc::new(Vec::new());
                self.search_open = false;
            }
            self.search_pending_at = None;
            self.search_sig = None;
        } else {
            let sig = (
                Arc::as_ptr(&model) as usize,
                root,
                weights_id,
                trimmed.clone(),
            );
            let due = self
                .search_pending_at
                .map(|at| at.elapsed() >= QUERY_DEBOUNCE)
                .unwrap_or(true);
            if self.search_sig.as_ref() != Some(&sig) && due {
                self.search_sig = Some(sig);
                self.search_job = self.alloc_job();
                self.search_generation
                    .store(self.search_job, Ordering::Release);
                self.search_open = true;
                self.search_pending_at = None;
                self.search_results = Arc::new(Vec::new());
                self.search_worker_pending = self
                    .query_sender
                    .send(QueryRequest::Search {
                        job: self.search_job,
                        cancel: self.search_generation.clone(),
                        model: model.clone(),
                        root,
                        query: trimmed,
                        matches: self.active_weights.as_ref().map(|w| w.matches.clone()),
                    })
                    .is_ok();
            }
        }
    }

    fn alloc_job(&mut self) -> u64 {
        self.next_job_id = self.next_job_id.wrapping_add(1).max(1);
        self.next_job_id
    }

    fn invalidate_query_jobs(&mut self) {
        let job = self.alloc_job();
        self.filter_job = job;
        self.weights_generation.store(job, Ordering::Release);
        self.weights_pending = false;
        let job = self.alloc_job();
        self.largest_job = job;
        self.largest_generation.store(job, Ordering::Release);
        self.largest_pending = false;
        let job = self.alloc_job();
        self.efficiency_job = job;
        self.efficiency_generation.store(job, Ordering::Release);
        self.efficiency_pending = false;
        let job = self.alloc_job();
        self.search_job = job;
        self.search_generation.store(job, Ordering::Release);
        self.search_worker_pending = false;
    }

    fn invalidate_dependent_query_jobs(&mut self) {
        let job = self.alloc_job();
        self.largest_job = job;
        self.largest_generation.store(job, Ordering::Release);
        self.largest_pending = false;
        let job = self.alloc_job();
        self.efficiency_job = job;
        self.efficiency_generation.store(job, Ordering::Release);
        self.efficiency_pending = false;
        let job = self.alloc_job();
        self.search_job = job;
        self.search_generation.store(job, Ordering::Release);
        self.search_worker_pending = false;
        self.largest_sig = None;
        self.efficiency_sig = None;
        self.search_sig = None;
    }

    fn clear_query_results(&mut self) {
        self.largest_rows = Arc::new(Vec::new());
        self.efficiency_result = None;
        self.search_results = Arc::new(Vec::new());
    }

    /// Delay until a pending debounced edit must schedule its worker job.
    pub fn query_repaint_after(&self) -> Option<Duration> {
        let now = Instant::now();
        [self.filter_pending_at, self.search_pending_at]
            .into_iter()
            .flatten()
            .map(|at| QUERY_DEBOUNCE.saturating_sub(now.saturating_duration_since(at)))
            .min()
    }

    pub fn has_background_work(&self) -> bool {
        self.phase == DiskAnalysisPhase::Fetching
            || self.weights_pending
            || self.largest_pending
            || self.efficiency_pending
            || self.search_worker_pending
            || self.duplicates.is_running()
    }

    pub fn filter_is_updating(&self) -> bool {
        self.filter.is_active() && self.active_weights.is_none()
    }

    /// Drain worker messages (fetch + queries + duplicates). Returns true
    /// when the view must repaint data.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(message) = self.res_receiver.try_recv() {
            changed |= self.apply_message(message);
        }
        while let Ok(message) = self.query_receiver.try_recv() {
            changed |= self.apply_query_message(message);
        }
        changed |= self.duplicates.poll();
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
                self.reset_view_state(false);
                self.treemap_cache.clear();
                self.drives = drives;
                self.fetch_elapsed = Some(fetch_elapsed);
                self.phase = DiskAnalysisPhase::Ready;
                self.largest_sig = None;
                self.efficiency_sig = None;
                self.search_sig = None;
                self.filter_scheduled_sig = None;
                self.filter_pending_at = Some(Instant::now());
                true
            }
            DiskAnalysisMessage::Failed { request_id, error } => {
                if request_id != self.active_request_id {
                    return false;
                }
                self.phase = DiskAnalysisPhase::Failed;
                self.error = Some(error);
                self.reset_view_state(true);
                self.largest_sig = None;
                self.efficiency_sig = None;
                self.search_sig = None;
                true
            }
        }
    }

    fn apply_query_message(&mut self, message: QueryMessage) -> bool {
        match message {
            QueryMessage::Weights {
                job,
                root,
                metric,
                weights,
                matches,
            } => {
                if job != self.filter_job {
                    return false;
                }
                self.weights_pending = false;
                self.active_weights = Some(ActiveWeights {
                    id: job,
                    root,
                    metric,
                    weights,
                    matches,
                });
                self.largest_sig = None;
                self.efficiency_sig = None;
                self.search_sig = None;
                true
            }
            QueryMessage::Largest { job, rows } => {
                if job != self.largest_job {
                    return false;
                }
                self.largest_pending = false;
                self.largest_rows = rows;
                self.sort_largest_rows();
                true
            }
            QueryMessage::Efficiency { job, result } => {
                if job != self.efficiency_job {
                    return false;
                }
                self.efficiency_pending = false;
                self.efficiency_result = Some(result);
                true
            }
            QueryMessage::Search { job, hits } => {
                if job != self.search_job {
                    return false;
                }
                self.search_worker_pending = false;
                self.search_results = hits;
                self.search_selected = 0;
                self.search_open = true;
                true
            }
        }
    }

    /// Sort the largest-rows projection by the current column/direction.
    /// Reads sizes straight from the model; rows are just indices.
    pub fn sort_largest_rows(&mut self) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let column = self.largest_sort_column;
        let asc = self.largest_sort_asc;
        let filtered = self.active_weights.clone();
        let filtered_value = |idx: u32| {
            filtered
                .as_ref()
                .and_then(|weights| weights.weights.get(idx as usize).copied())
        };
        let mut rows: Vec<u32> = self.largest_rows.as_ref().clone();
        rows.sort_by(|&a, &b| {
            let ord = match column {
                LargestColumn::Name => model.nodes[a as usize]
                    .name
                    .to_lowercase()
                    .cmp(&model.nodes[b as usize].name.to_lowercase()),
                LargestColumn::Path => model.path_of(a).cmp(&model.path_of(b)),
                LargestColumn::Logical if self.metric == SizeMetric::Logical => filtered_value(a)
                    .unwrap_or(model.nodes[a as usize].subtree_size)
                    .cmp(&filtered_value(b).unwrap_or(model.nodes[b as usize].subtree_size)),
                LargestColumn::Logical => model.nodes[a as usize]
                    .subtree_size
                    .cmp(&model.nodes[b as usize].subtree_size),
                LargestColumn::Allocated if self.metric == SizeMetric::Allocated => {
                    filtered_value(a)
                        .unwrap_or(model.nodes[a as usize].subtree_allocated_size)
                        .cmp(
                            &filtered_value(b)
                                .unwrap_or(model.nodes[b as usize].subtree_allocated_size),
                        )
                }
                LargestColumn::Allocated => model.nodes[a as usize]
                    .subtree_allocated_size
                    .cmp(&model.nodes[b as usize].subtree_allocated_size),
                LargestColumn::Difference => {
                    signed_difference(&model, a).cmp(&signed_difference(&model, b))
                }
                LargestColumn::Files if self.metric == SizeMetric::FileCount => filtered_value(a)
                    .unwrap_or(model.nodes[a as usize].subtree_files)
                    .cmp(&filtered_value(b).unwrap_or(model.nodes[b as usize].subtree_files)),
                LargestColumn::Files => model.nodes[a as usize]
                    .subtree_files
                    .cmp(&model.nodes[b as usize].subtree_files),
            };
            if asc {
                ord
            } else {
                ord.reverse()
            }
        });
        self.largest_rows = Arc::new(rows);
    }
}

/// Signed logical-minus-allocated difference safe against u64 overflow
/// (returns the value as f64 for ordering; display uses the split form).
fn signed_difference(model: &DiskAnalysisModel, idx: u32) -> i128 {
    let node = &model.nodes[idx as usize];
    node.subtree_size as i128 - node.subtree_allocated_size as i128
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

fn query_worker_loop(req_receiver: Receiver<QueryRequest>, res_sender: Sender<QueryMessage>) {
    while let Ok(request) = req_receiver.recv() {
        let job = request.job();
        let cancel = request.cancel().clone();
        let is_cancelled = || cancel.load(Ordering::Acquire) != job;
        if is_cancelled() {
            continue;
        }
        match request {
            QueryRequest::Weights {
                model,
                root,
                filter,
                metric,
                ..
            } => {
                if let Some(out) =
                    crate::app::disk_analysis_query::compute_filtered_weights_cancellable(
                        &model,
                        root,
                        &filter,
                        metric,
                        is_cancelled,
                    )
                {
                    let _ = res_sender.send(QueryMessage::Weights {
                        job,
                        root,
                        metric,
                        weights: Arc::new(out.weights),
                        matches: Arc::new(out.matches),
                    });
                }
            }
            QueryRequest::Largest {
                model,
                root,
                metric,
                weights,
                ..
            } => {
                let basis = basis_of(metric, weights.as_deref().map(|v| v.as_slice()));
                let rows = crate::app::disk_analysis_query::largest_items(
                    &model,
                    root,
                    basis,
                    LARGEST_LIMIT,
                    is_cancelled,
                );
                if !is_cancelled() {
                    let _ = res_sender.send(QueryMessage::Largest {
                        job,
                        rows: Arc::new(rows),
                    });
                }
            }
            QueryRequest::Efficiency {
                model,
                root,
                matches,
                ..
            } => {
                let result = crate::app::disk_analysis_query::efficiency_scan(
                    &model,
                    root,
                    matches.as_deref().map(|v| v.as_slice()),
                    EFFICIENCY_LIMIT,
                    is_cancelled,
                );
                if !is_cancelled() {
                    let _ = res_sender.send(QueryMessage::Efficiency {
                        job,
                        result: Arc::new(result),
                    });
                }
            }
            QueryRequest::Search {
                model,
                root,
                query,
                matches,
                ..
            } => {
                let hits = crate::app::disk_analysis_query::search_nodes(
                    &model,
                    root,
                    &query,
                    matches.as_deref().map(|v| v.as_slice()),
                    SEARCH_LIMIT,
                    is_cancelled,
                );
                if !is_cancelled() {
                    let _ = res_sender.send(QueryMessage::Search {
                        job,
                        hits: Arc::new(hits),
                    });
                }
            }
        }
    }
}

fn basis_of<'a>(
    metric: SizeMetric,
    weights: Option<&'a [u64]>,
) -> crate::app::disk_analysis_query::WeightBasis<'a> {
    match weights {
        Some(w) => crate::app::disk_analysis_query::WeightBasis::Filtered(w),
        None => crate::app::disk_analysis_query::WeightBasis::Metric(metric),
    }
}

impl QueryRequest {
    fn job(&self) -> u64 {
        match self {
            QueryRequest::Weights { job, .. }
            | QueryRequest::Largest { job, .. }
            | QueryRequest::Efficiency { job, .. }
            | QueryRequest::Search { job, .. } => *job,
        }
    }

    fn cancel(&self) -> &Arc<AtomicU64> {
        match self {
            QueryRequest::Weights { cancel, .. }
            | QueryRequest::Largest { cancel, .. }
            | QueryRequest::Efficiency { cancel, .. }
            | QueryRequest::Search { cancel, .. } => cancel,
        }
    }
}

/// Initial cap for the Largest items list (plan section 4).
const LARGEST_LIMIT: usize = 100;
/// Rows kept per efficiency group (full totals are computed regardless).
const EFFICIENCY_LIMIT: usize = 200;
/// Initial cap for search results (plan section 5).
const SEARCH_LIMIT: usize = 200;

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
        state.selected = Some(1);

        state.request('D');

        assert_eq!(state.drive_letter, Some('D'));
        assert!(state.model.is_none());
        assert!(state.drill_stack.is_empty());
        assert_eq!(state.selected, None);
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

    #[test]
    fn model_ready_resets_selection_and_queries() {
        let mut state = DiskAnalysisState::new();
        state.drive_letter = Some('C');
        state.active_request_id = 3;
        state.selected = Some(9);
        state.search_text = "abc".to_string();
        state.search_results = Arc::new(vec![1, 2]);

        assert!(state.apply_message(DiskAnalysisMessage::ModelReady {
            request_id: 3,
            model: model('C'),
            drives: Vec::new(),
            fetch_elapsed: Duration::ZERO,
        }));
        assert_eq!(state.selected, None);
        assert!(state.search_results.is_empty());
        assert!(!state.search_open);
    }

    #[test]
    fn stale_query_messages_are_discarded() {
        let mut state = DiskAnalysisState::new();
        state.largest_job = 42;
        state.apply_query_message(QueryMessage::Largest {
            job: 41,
            rows: Arc::new(vec![1]),
        });
        assert!(state.largest_rows.is_empty());
        state.apply_query_message(QueryMessage::Largest {
            job: 42,
            rows: Arc::new(vec![7]),
        });
        assert_eq!(state.largest_rows.as_ref(), &vec![7]);
    }

    #[test]
    fn model_reset_invalidates_all_query_messages_and_weights() {
        let mut state = DiskAnalysisState::new();
        let old_job = state.largest_job;
        state.active_weights = Some(ActiveWeights {
            id: state.filter_job,
            root: 0,
            metric: SizeMetric::Allocated,
            weights: Arc::new(vec![1]),
            matches: Arc::new(vec![true]),
        });

        state.reset_view_state(false);

        assert!(state.active_weights.is_none());
        assert!(!state.apply_query_message(QueryMessage::Largest {
            job: old_job,
            rows: Arc::new(vec![99]),
        }));
        assert!(state.largest_rows.is_empty());
    }

    #[test]
    fn scheduling_search_clears_debounce_deadline() {
        let mut state = DiskAnalysisState::new();
        let model = model('C');
        state.drill_stack = vec![model.root];
        state.model = Some(model);
        state.search_text = "file".to_string();
        state.search_pending_at = Some(Instant::now() - QUERY_DEBOUNCE);

        state.sync_query_jobs();

        assert!(state.search_pending_at.is_none());
        assert!(state.search_worker_pending);
        assert!(state.query_repaint_after().is_none());
    }

    #[test]
    fn clearing_search_clears_pending_worker_poll() {
        let mut state = DiskAnalysisState::new();
        state.search_text.clear();
        state.search_worker_pending = true;

        state.mark_search_changed();
        state.sync_query_jobs();

        assert!(!state.search_worker_pending);
        assert!(!state.has_background_work());
    }

    #[test]
    fn changed_query_context_clears_old_projections() {
        let mut state = DiskAnalysisState::new();
        let model = model('C');
        state.drill_stack = vec![model.root];
        state.model = Some(model);
        state.query_context = Some((usize::MAX, u32::MAX, SizeMetric::Logical));
        state.largest_rows = Arc::new(vec![77]);
        state.efficiency_result = Some(Arc::new(EfficiencyResult::default()));
        state.search_results = Arc::new(vec![88]);

        state.sync_query_jobs();

        assert!(state.largest_rows.is_empty());
        assert!(state.efficiency_result.is_none());
        assert!(state.search_results.is_empty());
    }

    #[test]
    fn reveal_file_drills_into_parent_and_selects() {
        let mut state = DiskAnalysisState::new();
        let m = Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
            drive_letter: 'C',
            records: vec![
                DiskAnalysisRecord {
                    frn: 5,
                    parent_frn: 5,
                    name: String::new(),
                    size: 0,
                    allocated_size: 0,
                    is_dir: true,
                    is_reparse: false,
                },
                DiskAnalysisRecord {
                    frn: 6,
                    parent_frn: 5,
                    name: "dir".to_string(),
                    size: 0,
                    allocated_size: 0,
                    is_dir: true,
                    is_reparse: false,
                },
                DiskAnalysisRecord {
                    frn: 7,
                    parent_frn: 6,
                    name: "file.txt".to_string(),
                    size: 10,
                    allocated_size: 10,
                    is_dir: false,
                    is_reparse: false,
                },
            ],
        }));
        state.model = Some(m.clone());
        state.drill_stack = vec![m.root];

        let file_idx = m.nodes.iter().position(|n| n.name == "file.txt").unwrap() as u32;
        state.search_text = "file".to_string();
        state.search_open = true;
        state.search_results = Arc::new(vec![file_idx]);
        crate::ui::disk_analysis::toolbar::activate_result(&mut state, file_idx);

        assert_eq!(state.selected, Some(file_idx));
        assert!(state.search_text.is_empty());
        assert!(!state.search_open);
        assert!(state.search_results.is_empty());
        assert_eq!(
            state.current_root(),
            Some(m.nodes[file_idx as usize].parent)
        );
    }

    #[test]
    fn invalid_search_result_activation_is_ignored() {
        let mut state = DiskAnalysisState::new();
        state.model = Some(model('C'));
        state.search_text = "stale result".to_string();
        state.search_open = true;
        state.search_results = Arc::new(vec![u32::MAX]);

        assert!(!crate::ui::disk_analysis::toolbar::activate_result(
            &mut state,
            u32::MAX
        ));
        assert_eq!(state.search_text, "stale result");
        assert!(state.search_open);
        assert_eq!(state.search_results.as_slice(), &[u32::MAX]);
        assert!(state.selected.is_none());
    }
}
