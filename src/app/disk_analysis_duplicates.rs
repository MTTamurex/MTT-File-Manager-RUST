//! Duplicate file detection for the disk analyzer (plan section 7).
//!
//! Two-stage scan started only by explicit user action:
//! 1. **Candidates** — regular files under the captured subtree are grouped
//!    by logical size; singletons and empty files are dropped, and a
//!    configurable minimum size (default 1 MiB) keeps the workload sane.
//!    Reparse points are never traversed.
//! 2. **Confirmation** — every candidate gets a full BLAKE3 hash, read with
//!    sequential access, shared delete/write/read modes and low I/O priority.
//!    File identity (volume serial + file index), size and timestamps are
//!    captured from the handle before and after the read; anything that
//!    changed mid-read is discarded instead of trusted. Size alone is never
//!    treated as proof of duplication.
//!
//! Hardlinks share one physical allocation: members carrying the same file
//! identity collapse to their largest single allocation when computing
//! reclaimable space, and groups whose members all resolve to one inode are
//! reported only as a statistic (`hardlink_only_groups`).
//!
//! The scan cancels on drive switch, rescan, model change, panel action or
//! window teardown (see [`DuplicateSession::cancel`]). Cloud placeholders
//! that are not locally available are never opened.

use crate::app::disk_analysis_model::DiskAnalysisModel;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

/// Default minimum candidate size: 1 MiB.
pub const DEFAULT_MIN_SIZE: u64 = 1024 * 1024;
/// Read chunk used while hashing (sequential-friendly).
const READ_CHUNK: usize = 1024 * 1024;
static VOLUME_SCAN_LOCKS: OnceLock<[Mutex<()>; 26]> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DuplicatePhase {
    #[default]
    Idle,
    Collecting,
    Hashing,
    Finalizing,
    Complete,
    Partial,
    Cancelled,
    Failed,
}

/// Live counters shown while the scan runs.
#[derive(Clone, Debug, Default)]
pub struct DuplicateProgress {
    pub processed_files: u64,
    pub total_candidates: u64,
    pub hashed_bytes: u64,
    /// Node currently being hashed (path resolved lazily by the UI).
    pub current_idx: Option<u32>,
    pub inaccessible: u64,
    pub changed_during_read: u64,
    /// Cloud placeholders / unavailable-on-demand files left untouched.
    pub skipped_unavailable: u64,
    pub skipped_by_limit: u64,
}

/// One member of a duplicate group (projection over the model).
#[derive(Clone, Debug)]
pub struct DuplicateMember {
    pub idx: u32,
    pub logical_size: u64,
    pub allocated_size: u64,
}

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub logical_size: u64,
    pub members: Vec<DuplicateMember>,
    /// Conservatively reclaimable bytes: every distinct physical copy minus
    /// the largest kept copy. Hardlinked copies contribute zero.
    pub recoverable: u64,
    pub has_hardlinks: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DuplicateStats {
    pub total_candidates: u64,
    pub processed_files: u64,
    pub hashed_bytes: u64,
    pub inaccessible: u64,
    pub changed_during_read: u64,
    pub skipped_unavailable: u64,
    pub skipped_by_limit: u64,
    /// Groups whose members were all the same physical file (hardlinks).
    pub hardlink_only_groups: u64,
}

#[derive(Clone, Debug)]
pub struct DuplicateReport {
    pub groups: Vec<DuplicateGroup>,
    pub total_recoverable: u64,
    pub stats: DuplicateStats,
}

#[derive(Debug)]
pub(crate) enum DuplicateFinish {
    Complete(Box<DuplicateReport>),
    Partial(Box<DuplicateReport>),
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
pub(crate) enum DuplicateMessage {
    Progress(DuplicatePhase, DuplicateProgress),
    Finished(DuplicateFinish),
}

pub(crate) type PathResolver = Arc<dyn Fn(u32) -> Option<PathBuf> + Send + Sync>;

/// UI-side handle for one duplicate-detection session.
pub struct DuplicateSession {
    pub phase: DuplicatePhase,
    /// Minimum candidate size text (bytes), editable between runs.
    pub min_size_text: String,
    pub progress: DuplicateProgress,
    pub report: Option<Arc<DuplicateReport>>,
    cancel_flag: Option<Arc<AtomicBool>>,
    receiver: Option<Receiver<DuplicateMessage>>,
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
            progress: DuplicateProgress::default(),
            report: None,
            cancel_flag: None,
            receiver: None,
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.phase,
            DuplicatePhase::Collecting | DuplicatePhase::Hashing | DuplicatePhase::Finalizing
        )
    }

    pub fn min_size(&self) -> Option<u64> {
        self.min_size_text
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
    }

    /// Start a scan over `root`'s subtree using `model` for walking and path
    /// resolution. Any previous scan is cancelled first.
    pub fn start(&mut self, model: Arc<DiskAnalysisModel>, root: u32) {
        let resolver: PathResolver = {
            let model = model.clone();
            Arc::new(move |idx: u32| Some(PathBuf::from(model.path_of(idx))))
        };
        self.start_with_resolver(model, root, resolver);
    }

    /// Same as [`start`](Self::start) but with a caller-supplied path
    /// resolver (used by tests to point candidates at temporary files).
    pub(crate) fn start_with_resolver(
        &mut self,
        model: Arc<DiskAnalysisModel>,
        root: u32,
        resolver: PathResolver,
    ) {
        self.cancel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let Some(min_size) = self.min_size() else {
            return;
        };
        self.cancel_flag = Some(cancel_flag.clone());
        self.receiver = Some(rx);
        self.report = None;
        self.progress = DuplicateProgress::default();
        self.phase = DuplicatePhase::Collecting;
        let _ = std::thread::Builder::new()
            .name("disk-analysis-duplicates".to_string())
            .spawn(move || run_scan(model, root, min_size, resolver, cancel_flag, tx));
    }

    /// Request cancellation. Safe to call any time; idempotent.
    pub fn cancel(&mut self) {
        if let Some(flag) = &self.cancel_flag {
            flag.store(true, Ordering::Release);
        }
        self.cancel_flag = None;
        // Each run owns its channel. Dropping it is the generation boundary:
        // no late progress or finish from the cancelled run can be applied.
        self.receiver = None;
        self.progress = DuplicateProgress::default();
        if self.is_running() {
            // The worker will also observe the flag; reflect it right away so
            // the UI does not offer a stale "Cancel" button.
            self.phase = DuplicatePhase::Cancelled;
        }
    }

    /// Cancel the current run and clear all model-derived output.
    pub fn invalidate(&mut self) {
        self.cancel();
        self.phase = DuplicatePhase::Idle;
        self.progress = DuplicateProgress::default();
        self.report = None;
    }

    /// Drain worker messages. Returns true when something visible changed.
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
                    changed = true;
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
                        DuplicateFinish::Cancelled => {
                            self.phase = DuplicatePhase::Cancelled;
                        }
                        DuplicateFinish::Failed(error) => {
                            self.phase = DuplicatePhase::Failed;
                            log::error!("[DISK-ANALYZER] duplicate scan failed: {error}");
                        }
                    }
                    changed = true;
                }
            }
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

/// Physical identity captured from an open handle (volume serial + file
/// index). Hardlinked names share it.
pub type FileIdentity = [u64; 2];

enum CandidateOutcome {
    Hashed {
        digest: [u8; 32],
        identity: Option<FileIdentity>,
    },
    Inaccessible,
    Unavailable,
    ChangedDuringRead,
    Cancelled,
}

/// Group candidate `(idx, size)` pairs by identical logical size, dropping
/// singleton groups. Output is ordered by ascending size (small files hash
/// first so early progress is fast); members stay sorted by ascending index.
pub fn group_by_size(candidates: Vec<(u32, u64)>) -> Vec<(u64, Vec<u32>)> {
    group_by_size_cancellable(candidates, || false).expect("non-cancellable grouping")
}

fn group_by_size_cancellable(
    candidates: Vec<(u32, u64)>,
    is_cancelled: impl Fn() -> bool,
) -> Option<Vec<(u64, Vec<u32>)>> {
    let mut by_size: HashMap<u64, Vec<u32>> = HashMap::new();
    for (idx, size) in candidates {
        if is_cancelled() {
            return None;
        }
        by_size.entry(size).or_default().push(idx);
    }
    let mut groups: Vec<(u64, Vec<u32>)> = by_size.into_iter().collect();
    for (_, members) in groups.iter_mut() {
        members.sort_unstable();
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups.retain(|(_, members)| members.len() > 1);
    Some(groups)
}

/// Conservative reclaimable-space math for one confirmed group.
///
/// `entries` holds `(physical identity, allocated bytes)` per member. Copies
/// sharing an identity are the same physical file: they collapse to their
/// largest single allocation. With two or more distinct identities the
/// reclaimable space is everything above keeping just the largest copy.
/// Returns `(recoverable, has_hardlinks)`.
pub fn compute_recoverable(entries: &[(FileIdentity, u64)]) -> (u64, bool) {
    let mut best_per_identity: HashMap<FileIdentity, u64> = HashMap::new();
    for &(id, allocated) in entries {
        let slot = best_per_identity.entry(id).or_insert(0);
        *slot = (*slot).max(allocated);
    }
    let has_hardlinks = best_per_identity.len() < entries.len();
    if best_per_identity.len() < 2 {
        return (0, has_hardlinks);
    }
    let total: u64 = best_per_identity.values().copied().sum();
    let max = best_per_identity.values().copied().max().unwrap_or(0);
    (total.saturating_sub(max), has_hardlinks)
}

/// Full pipeline: collect → hash → report. Runs on its own thread; progress
/// flows through `tx` and termination always ends with exactly one message.
fn run_scan(
    model: Arc<DiskAnalysisModel>,
    root: u32,
    min_size: u64,
    resolver: PathResolver,
    cancel: Arc<AtomicBool>,
    tx: Sender<DuplicateMessage>,
) {
    // Defensive guard: a stale session could hand us a root from another
    // model; that is a hard failure instead of an empty report.
    if root as usize >= model.nodes.len() {
        let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Failed(
            "duplicate scan root outside captured model".to_string(),
        )));
        return;
    }
    let send_progress =
        |phase: DuplicatePhase, p: &DuplicateProgress, tx: &Sender<DuplicateMessage>| {
            let _ = tx.send(DuplicateMessage::Progress(phase, p.clone()));
        };

    // ---- Stage 1: candidates ------------------------------------------
    let mut candidates: Vec<(u32, u64)> = Vec::new();
    let mut skipped_by_limit = 0u64;
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if cancel.load(Ordering::Acquire) {
            let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
            return;
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir {
            if !node.is_reparse {
                for &child in model.children(idx) {
                    stack.push(child);
                }
            }
            continue;
        }
        // Regular files only; never follow a file reparse point to a target
        // outside the captured subtree.
        if node.is_reparse {
            continue;
        }
        if node.size > 0 && node.size >= min_size {
            candidates.push((idx, node.size));
        } else {
            skipped_by_limit += 1;
        }
    }

    let Some(groups) = group_by_size_cancellable(candidates, || cancel.load(Ordering::Acquire))
    else {
        let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
        return;
    };
    let total_candidates: u64 = groups.iter().map(|(_, m)| m.len() as u64).sum();
    let mut progress = DuplicateProgress {
        total_candidates,
        skipped_by_limit,
        ..DuplicateProgress::default()
    };
    send_progress(DuplicatePhase::Hashing, &progress, &tx);
    if total_candidates == 0 {
        let report = Box::new(DuplicateReport {
            groups: Vec::new(),
            total_recoverable: 0,
            stats: DuplicateStats {
                skipped_by_limit,
                ..DuplicateStats::default()
            },
        });
        let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Complete(
            report,
        )));
        return;
    }

    // Multiple analyzer sessions may briefly overlap during cancellation.
    // Serialize confirmation reads per drive without blocking the UI thread.
    let locks = VOLUME_SCAN_LOCKS.get_or_init(|| std::array::from_fn(|_| Mutex::new(())));
    let drive_slot = model.drive_letter.to_ascii_uppercase() as usize - 'A' as usize;
    let _volume_guard = locks[drive_slot.min(25)]
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if cancel.load(Ordering::Acquire) {
        let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
        return;
    }

    // ---- Stage 2: hashing ---------------------------------------------
    let mut hashes: HashMap<u32, ([u8; 32], Option<FileIdentity>)> = HashMap::new();
    'outer: for (_size, members) in &groups {
        for &idx in members {
            if cancel.load(Ordering::Acquire) {
                let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
                return;
            }
            progress.current_idx = Some(idx);
            send_progress(DuplicatePhase::Hashing, &progress, &tx);
            let mut bytes_since_update = 0u64;
            let expected_size = model.nodes[idx as usize].size;
            let outcome = hash_candidate(&resolver, idx, expected_size, &cancel, |bytes| {
                progress.hashed_bytes = progress.hashed_bytes.saturating_add(bytes);
                bytes_since_update = bytes_since_update.saturating_add(bytes);
                if bytes_since_update >= 16 * 1024 * 1024 {
                    send_progress(DuplicatePhase::Hashing, &progress, &tx);
                    bytes_since_update = 0;
                }
            });
            match outcome {
                CandidateOutcome::Hashed { digest, identity } => {
                    hashes.insert(idx, (digest, identity));
                    progress.processed_files += 1;
                }
                CandidateOutcome::Inaccessible => {
                    progress.processed_files += 1;
                    progress.inaccessible += 1;
                }
                CandidateOutcome::Unavailable => {
                    progress.processed_files += 1;
                    progress.skipped_unavailable += 1;
                }
                CandidateOutcome::ChangedDuringRead => {
                    progress.processed_files += 1;
                    progress.changed_during_read += 1;
                }
                CandidateOutcome::Cancelled => break 'outer,
            }
            if progress.processed_files.is_multiple_of(8)
                || progress.processed_files == progress.total_candidates
            {
                send_progress(DuplicatePhase::Hashing, &progress, &tx);
            }
        }
    }
    if cancel.load(Ordering::Acquire) {
        let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
        return;
    }

    // ---- Stage 3: report ----------------------------------------------
    progress.current_idx = None;
    send_progress(DuplicatePhase::Finalizing, &progress, &tx);

    let mut out_groups: Vec<DuplicateGroup> = Vec::new();
    let mut hardlink_only_groups: u64 = 0;
    for (size, members) in &groups {
        if cancel.load(Ordering::Acquire) {
            let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
            return;
        }
        let mut keyed: HashMap<[u8; 32], Vec<u32>> = HashMap::new();
        for &idx in members {
            if cancel.load(Ordering::Acquire) {
                let _ = tx.send(DuplicateMessage::Finished(DuplicateFinish::Cancelled));
                return;
            }
            if let Some((digest, _)) = hashes.get(&idx) {
                keyed.entry(*digest).or_default().push(idx);
            }
        }
        for mut same_hash in keyed.into_values() {
            if same_hash.len() < 2 {
                continue; // unique content despite matching size
            }
            same_hash.sort_unstable();
            let entries: Vec<(FileIdentity, u64)> = same_hash
                .iter()
                .filter_map(|&idx| {
                    let (_, identity) = hashes.get(&idx)?;
                    let identity = (*identity)?;
                    Some((identity, model.nodes[idx as usize].allocated_size))
                })
                .collect();
            let distinct_identities: HashSet<FileIdentity> =
                entries.iter().map(|(identity, _)| *identity).collect();
            if distinct_identities.len() < 2 {
                if entries.len() >= 2 {
                    hardlink_only_groups += 1;
                }
                continue;
            }
            let (recoverable, has_hardlinks) = compute_recoverable(&entries);
            let members_out: Vec<DuplicateMember> = same_hash
                .iter()
                .map(|&idx| {
                    let node = &model.nodes[idx as usize];
                    DuplicateMember {
                        idx,
                        logical_size: node.size,
                        allocated_size: node.allocated_size,
                    }
                })
                .collect();
            out_groups.push(DuplicateGroup {
                logical_size: *size,
                members: members_out,
                recoverable,
                has_hardlinks,
            });
        }
    }
    out_groups.sort_by(|a, b| {
        b.recoverable
            .cmp(&a.recoverable)
            .then(b.logical_size.cmp(&a.logical_size))
    });
    let total_recoverable = out_groups.iter().map(|g| g.recoverable).sum::<u64>();

    let stats = DuplicateStats {
        total_candidates: progress.total_candidates,
        processed_files: progress.processed_files,
        hashed_bytes: progress.hashed_bytes,
        inaccessible: progress.inaccessible,
        changed_during_read: progress.changed_during_read,
        skipped_unavailable: progress.skipped_unavailable,
        skipped_by_limit: progress.skipped_by_limit,
        hardlink_only_groups,
    };
    let report = Box::new(DuplicateReport {
        groups: out_groups,
        total_recoverable,
        stats,
    });
    let degraded =
        stats.inaccessible > 0 || stats.changed_during_read > 0 || stats.skipped_unavailable > 0;
    let finish = if degraded {
        DuplicateFinish::Partial(report)
    } else {
        DuplicateFinish::Complete(report)
    };
    let _ = tx.send(DuplicateMessage::Finished(finish));
}

fn hash_candidate(
    resolver: &PathResolver,
    idx: u32,
    expected_size: u64,
    cancel: &Arc<AtomicBool>,
    on_read: impl FnMut(u64),
) -> CandidateOutcome {
    let Some(path) = resolver(idx) else {
        return CandidateOutcome::Inaccessible;
    };
    #[cfg(windows)]
    {
        hash_candidate_windows(&path, expected_size, cancel, on_read)
    }
    #[cfg(not(windows))]
    {
        hash_candidate_portable(&path, idx, expected_size, cancel, on_read)
    }
}

#[cfg(not(windows))]
fn hash_candidate_portable(
    path: &Path,
    idx: u32,
    expected_size: u64,
    cancel: &Arc<AtomicBool>,
    mut on_read: impl FnMut(u64),
) -> CandidateOutcome {
    use std::io::Read;
    match std::fs::File::open(path) {
        Ok(mut file) => {
            let before = file.metadata().ok();
            let mut hasher = blake3::Hasher::new();
            let mut buffer = vec![0u8; READ_CHUNK];
            let mut total = 0u64;
            loop {
                if cancel.load(Ordering::Acquire) {
                    return CandidateOutcome::Cancelled;
                }
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        hasher.update(&buffer[..n]);
                        total += n as u64;
                        on_read(n as u64);
                    }
                    Err(_) => return CandidateOutcome::Inaccessible,
                }
            }
            let after = file.metadata().ok();
            if total != expected_size
                || before.as_ref().map(|m| (m.len(), m.modified().ok()))
                    != after.as_ref().map(|m| (m.len(), m.modified().ok()))
            {
                return CandidateOutcome::ChangedDuringRead;
            }
            let digest: [u8; 32] = hasher.finalize().into();
            CandidateOutcome::Hashed {
                digest,
                identity: Some([u64::MAX, idx as u64]),
            }
        }
        Err(_) => CandidateOutcome::Inaccessible,
    }
}

#[cfg(windows)]
mod win_hash {
    use super::{CandidateOutcome, FileIdentity, READ_CHUNK};
    use std::io::Read;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        self as fs, FileIoPriorityHintInfo, GetFileAttributesW, GetFileInformationByHandle,
        IoPriorityHintLow, SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
        FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_IO_PRIORITY_HINT_INFO,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, PRIORITY_HINT,
    };

    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    fn attributes(path: &Path) -> Option<u32> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let attrs = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
        (attrs != fs::INVALID_FILE_ATTRIBUTES).then_some(attrs)
    }

    /// Cloud placeholders must not be opened: touching them would trigger a
    /// full download just to hash content the user may not have locally.
    fn is_cloud_placeholder(path: &Path) -> bool {
        let Some(attrs) = attributes(path) else {
            return false; // let the open attempt surface the real problem
        };
        (attrs & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0) != 0
            || (attrs & FILE_ATTRIBUTE_RECALL_ON_OPEN.0) != 0
            || (attrs & FILE_ATTRIBUTE_OFFLINE.0) != 0
    }

    fn path_contains_reparse_point(path: &Path) -> bool {
        path.ancestors().any(|component| {
            attributes(component).is_some_and(|attrs| attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
        })
    }
    struct HandleInfo {
        volume_serial: u32,
        file_index_high: u32,
        file_index_low: u32,
        size: u64,
        creation: u64,
        last_write: u64,
    }

    impl HandleInfo {
        fn capture(handle: HANDLE) -> Option<Self> {
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            unsafe { GetFileInformationByHandle(handle, &mut info) }.ok()?;
            Some(Self {
                volume_serial: info.dwVolumeSerialNumber,
                file_index_high: info.nFileIndexHigh,
                file_index_low: info.nFileIndexLow,
                size: ((info.nFileSizeHigh as u64) << 32) | info.nFileSizeLow as u64,
                creation: filetime(&info, true),
                last_write: filetime(&info, false),
            })
        }

        fn identity(&self) -> FileIdentity {
            [
                self.volume_serial as u64,
                ((self.file_index_high as u64) << 32) | self.file_index_low as u64,
            ]
        }

        /// True while nothing observable changed between two captures:
        /// identity, size and timestamps must all match.
        fn same_as(&self, other: &Self) -> bool {
            self.volume_serial == other.volume_serial
                && self.file_index_high == other.file_index_high
                && self.file_index_low == other.file_index_low
                && self.size == other.size
                && self.creation == other.creation
                && self.last_write == other.last_write
        }
    }

    fn filetime(info: &BY_HANDLE_FILE_INFORMATION, creation: bool) -> u64 {
        let ft = if creation {
            &info.ftCreationTime
        } else {
            &info.ftLastWriteTime
        };
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    }

    fn open_shared_sequential(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0)
            .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    /// Best-effort low I/O priority so background hashing never starves the
    /// user's interactive workloads on the same volume.
    fn lower_io_priority(file: &std::fs::File) {
        let hint = FILE_IO_PRIORITY_HINT_INFO {
            PriorityHint: PRIORITY_HINT(IoPriorityHintLow.0),
        };
        unsafe {
            let _ = SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileIoPriorityHintInfo,
                &hint as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<FILE_IO_PRIORITY_HINT_INFO>() as u32,
            );
        }
    }

    pub(super) fn hash_candidate_windows(
        path: &Path,
        expected_size: u64,
        cancel: &Arc<AtomicBool>,
        mut on_read: impl FnMut(u64),
    ) -> CandidateOutcome {
        if is_cloud_placeholder(path) {
            return CandidateOutcome::Unavailable;
        }
        if path_contains_reparse_point(path) {
            return CandidateOutcome::ChangedDuringRead;
        }
        let file = match open_shared_sequential(path) {
            Ok(f) => f,
            Err(_) => return CandidateOutcome::Inaccessible,
        };
        let mut file = file;
        lower_io_priority(&file);
        let handle = HANDLE(file.as_raw_handle());
        let Some(before) = HandleInfo::capture(handle) else {
            return CandidateOutcome::Inaccessible;
        };
        if before.size != expected_size {
            return CandidateOutcome::ChangedDuringRead;
        }

        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; READ_CHUNK];
        let mut total = 0u64;
        loop {
            if cancel.load(Ordering::Acquire) {
                return CandidateOutcome::Cancelled;
            }
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    hasher.update(&buffer[..n]);
                    total += n as u64;
                    on_read(n as u64);
                }
                Err(_) => return CandidateOutcome::Inaccessible,
            }
        }

        let Some(after) = HandleInfo::capture(HANDLE(file.as_raw_handle())) else {
            return CandidateOutcome::Inaccessible;
        };
        if !before.same_as(&after) || total != before.size {
            return CandidateOutcome::ChangedDuringRead;
        }
        CandidateOutcome::Hashed {
            digest: hasher.finalize().into(),
            identity: Some(before.identity()),
        }
    }
}

#[cfg(windows)]
use win_hash::hash_candidate_windows;

#[cfg(test)]
mod tests {
    use super::*;
    use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

    fn rec(frn: u64, parent_frn: u64, name: &str, size: u64, is_dir: bool) -> DiskAnalysisRecord {
        DiskAnalysisRecord {
            frn,
            parent_frn,
            name: name.to_string(),
            size,
            allocated_size: size.next_power_of_two().max(size),
            is_dir,
            is_reparse: false,
        }
    }

    fn build_model(records: Vec<DiskAnalysisRecord>) -> Arc<DiskAnalysisModel> {
        Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
            drive_letter: 'C',
            records,
        }))
    }

    #[test]
    fn grouping_keeps_only_multi_file_size_classes_sorted() {
        let groups = group_by_size(vec![
            (7, 100),
            (3, 50),
            (9, 100),
            (1, 50),
            (2, 50),
            (5, 200),
        ]);
        // Size 200 appears once → dropped. Sizes ascend; members ascend.
        assert_eq!(groups, vec![(50, vec![1, 2, 3]), (100, vec![7, 9])]);
    }

    #[test]
    fn recoverable_math_is_conservative_and_hardlink_aware() {
        // Three distinct copies of 100/200/300 allocated → keep the largest.
        let (rec, links) = compute_recoverable(&[([1, 1], 100), ([2, 2], 200), ([3, 3], 300)]);
        assert_eq!(rec, 300); // 100 + 200 freed, largest (300) stays
        assert!(!links);

        // Two paths pointing at the SAME inode count once.
        let (rec, links) = compute_recoverable(&[([1, 1], 500), ([1, 1], 500)]);
        assert_eq!(rec, 0);
        assert!(links);

        // One hardlink pair plus a real third copy.
        let (rec, links) = compute_recoverable(&[([1, 1], 400), ([1, 1], 400), ([9, 9], 400)]);
        assert_eq!(rec, 400);
        assert!(links);

        // Distinct zero-allocation files are duplicates, not hardlink-only.
        let (rec, links) = compute_recoverable(&[([1, 1], 0), ([2, 2], 0)]);
        assert_eq!(rec, 0);
        assert!(!links);
    }

    #[test]
    fn full_scan_groups_same_content_and_skips_different_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a_path = dir.path().join("a.bin");
        let b_path = dir.path().join("b.bin");
        let c_path = dir.path().join("c.bin");
        std::fs::write(&a_path, vec![7u8; 4096]).unwrap();
        std::fs::write(&b_path, vec![7u8; 4096]).unwrap();
        std::fs::write(&c_path, vec![9u8; 4096]).unwrap();

        // Tree: root(5) → a(idx1), b(idx2), c(idx3). Sizes above the 1 KiB floor.
        let model = build_model(vec![
            rec(5, 5, "", 0, true),
            rec(6, 5, "a", 4096, false),
            rec(7, 5, "b", 4096, false),
            rec(8, 5, "c", 4096, false),
        ]);
        let dir_path: PathBuf = dir.path().to_path_buf();
        // Node indices: synthetic root is 0, the volume-root record lands at
        // index 1, so the three files are nodes 2, 3 and 4.
        let resolver: PathResolver = Arc::new(move |idx| {
            Some(dir_path.join(match idx {
                2 => "a.bin",
                3 => "b.bin",
                4 => "c.bin",
                _ => return None,
            }))
        });

        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        run_scan(model.clone(), model.root, 1024, resolver, cancel, tx);

        let mut saw_report = None;
        while let Ok(message) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            if let DuplicateMessage::Finished(finish) = message {
                match finish {
                    DuplicateFinish::Complete(report) => saw_report = Some(*report),
                    other => panic!("unexpected finish: {other:?}"),
                }
                break;
            }
        }
        let report = saw_report.expect("report received");

        // Exactly one real duplicate group {a,b}; c shares the size but not
        // the content, proving size alone is not confirmation.
        assert_eq!(report.groups.len(), 1);
        let group = &report.groups[0];
        let member_names: Vec<&str> = group
            .members
            .iter()
            .map(|m| match m.idx {
                2 => "a",
                3 => "b",
                _ => "?",
            })
            .collect();
        assert_eq!(member_names, vec!["a", "b"]);
        assert!(group.recoverable > 0);
        assert_eq!(report.stats.inaccessible, 0);
        assert_eq!(report.stats.changed_during_read, 0);
    }

    #[test]
    fn cancellation_produces_cancelled_finish() {
        let model = build_model(vec![
            rec(5, 5, "", 0, true),
            rec(6, 5, "x", 4096, false),
            rec(7, 5, "y", 4096, false),
        ]);
        let resolver: PathResolver = Arc::new(|_| None);
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(true)); // cancelled up front
        run_scan(model.clone(), model.root, 1024, resolver, cancel, tx);
        loop {
            match rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("finish")
            {
                DuplicateMessage::Progress(..) => {}
                DuplicateMessage::Finished(finish) => {
                    assert!(matches!(finish, DuplicateFinish::Cancelled));
                    break;
                }
            }
        }
    }

    #[test]
    fn minimum_size_requires_a_positive_integer() {
        let mut session = DuplicateSession::new();
        assert_eq!(session.min_size(), Some(DEFAULT_MIN_SIZE));
        session.min_size_text = "0".to_string();
        assert_eq!(session.min_size(), None);
        session.min_size_text = "invalid".to_string();
        assert_eq!(session.min_size(), None);
    }

    #[test]
    fn cancelling_session_drops_late_messages() {
        let mut session = DuplicateSession::new();
        let (tx, rx) = channel();
        let flag = Arc::new(AtomicBool::new(false));
        session.phase = DuplicatePhase::Hashing;
        session.progress = DuplicateProgress {
            processed_files: 7,
            total_candidates: 10,
            hashed_bytes: 4096,
            ..DuplicateProgress::default()
        };
        session.cancel_flag = Some(flag.clone());
        session.receiver = Some(rx);

        session.cancel();

        assert!(flag.load(Ordering::Acquire));
        assert_eq!(session.phase, DuplicatePhase::Cancelled);
        assert_eq!(session.progress.processed_files, 0);
        assert_eq!(session.progress.total_candidates, 0);
        assert_eq!(session.progress.hashed_bytes, 0);
        assert!(session.receiver.is_none());
        assert!(tx
            .send(DuplicateMessage::Finished(DuplicateFinish::Failed(
                "late".to_string()
            )))
            .is_err());
        assert!(!session.poll());
    }

    #[test]
    fn empty_subtree_reports_complete_empty() {
        let model = build_model(vec![rec(5, 5, "", 0, true)]);
        let resolver: PathResolver = Arc::new(|_| None);
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        run_scan(model.clone(), model.root, 1024, resolver, cancel, tx);
        loop {
            match rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .expect("finish")
            {
                DuplicateMessage::Finished(DuplicateFinish::Complete(report)) => {
                    assert!(report.groups.is_empty());
                    assert_eq!(report.total_recoverable, 0);
                    break;
                }
                DuplicateMessage::Progress(..) => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}
