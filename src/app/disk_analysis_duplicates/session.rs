use super::*;
use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};

pub struct DuplicateSession {
    pub phase: DuplicatePhase,
    pub min_size_text: String,
    pub read_mode: DuplicateReadMode,
    pub progress: DuplicateProgress,
    pub report: Option<Arc<DuplicateReport>>,
    pub(super) cancel_flag: Option<Arc<AtomicBool>>,
    pub(super) receiver: Option<Receiver<DuplicateMessage>>,
}

impl Default for DuplicateSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DuplicateSession {
    pub fn new() -> Self {
        Self {
            phase: DuplicatePhase::Idle,
            min_size_text: DEFAULT_MIN_SIZE.to_string(),
            read_mode: DuplicateReadMode::default(),
            progress: DuplicateProgress::default(),
            report: None,
            cancel_flag: None,
            receiver: None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.phase,
            DuplicatePhase::Collecting
                | DuplicatePhase::Comparing
                | DuplicatePhase::Hashing
                | DuplicatePhase::Finalizing
        )
    }

    pub fn min_size(&self) -> Option<u64> {
        self.min_size_text
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|v| *v > 0)
    }

    pub fn start(&mut self, model: Arc<DiskAnalysisModel>, root: u32) {
        let resolver: PathResolver = {
            let model = model.clone();
            Arc::new(move |idx| Some(PathBuf::from(model.path_of(idx))))
        };
        self.start_with_resolver(model, root, resolver);
    }

    pub(crate) fn start_with_resolver(
        &mut self,
        model: Arc<DiskAnalysisModel>,
        root: u32,
        resolver: PathResolver,
    ) {
        self.cancel();
        let Some(min_size) = self.min_size() else {
            return;
        };
        let flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        self.cancel_flag = Some(flag.clone());
        self.receiver = Some(rx);
        self.report = None;
        self.progress = DuplicateProgress::default();
        self.phase = DuplicatePhase::Collecting;
        let limits = self.read_mode.limits();
        if let Err(error) = std::thread::Builder::new()
            .name("disk-analysis-duplicates".into())
            .spawn(move || pipeline::run_scan(model, root, min_size, resolver, flag, tx, limits))
        {
            self.phase = DuplicatePhase::Failed;
            self.cancel_flag = None;
            self.receiver = None;
            log::error!("[DISK-ANALYZER] cannot start duplicate worker: {error}");
        }
    }

    pub fn cancel(&mut self) {
        if let Some(flag) = self.cancel_flag.take() {
            flag.store(true, Ordering::Release);
        }
        // Separate channels are the stale-session boundary.
        self.receiver = None;
        self.progress = DuplicateProgress::default();
        if self.is_running() {
            self.phase = DuplicatePhase::Cancelled;
        }
    }

    pub fn invalidate(&mut self) {
        self.cancel();
        self.phase = DuplicatePhase::Idle;
        self.report = None;
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.receiver else {
            return false;
        };
        let mut changed = false;
        while let Ok(message) = rx.try_recv() {
            match message {
                DuplicateMessage::Progress(phase, progress) => {
                    if self.is_running() {
                        self.phase = phase;
                    }
                    self.progress = progress;
                }
                DuplicateMessage::Finished(finish) => {
                    self.cancel_flag = None;
                    match finish {
                        DuplicateFinish::Complete(report) => {
                            self.phase = DuplicatePhase::Complete;
                            self.report = Some(Arc::from(report));
                        }
                        DuplicateFinish::Partial(report) => {
                            self.phase = DuplicatePhase::Partial;
                            self.report = Some(Arc::from(report));
                        }
                        DuplicateFinish::Cancelled => self.phase = DuplicatePhase::Cancelled,
                        DuplicateFinish::Failed(error) => {
                            self.phase = DuplicatePhase::Failed;
                            log::error!("[DISK-ANALYZER] duplicate scan failed: {error}");
                        }
                    }
                }
            }
            changed = true;
        }
        changed
    }
}

impl Drop for DuplicateSession {
    fn drop(&mut self) {
        if let Some(flag) = &self.cancel_flag {
            flag.store(true, Ordering::Release);
        }
    }
}
