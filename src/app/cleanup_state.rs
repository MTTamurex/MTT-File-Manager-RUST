//! Cleanup feature state
//!
//! Owns the lifecycle of a cleanup run (scan → review → delete → summary)
//! plus the persisted run history. Channels follow the app convention: the
//! scan worker sends `CleanupScanMessage`s on a std mpsc channel drained once
//! per frame; delete progress arrives through the shared file-operation
//! result channel.

use crate::domain::cleanup::{
    CleanupCandidate, CleanupDeleteMode, CleanupDeleteProgress, CleanupPhase, CleanupRunSummary,
};
use rustc_hash::FxHashSet;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc::Receiver, Arc};

/// Default age filter: only entries untouched for at least a day.
pub const DEFAULT_CLEANUP_AGE_HOURS: u64 = 24;

/// Selectable age filters (hours) offered in the review UI.
pub const CLEANUP_AGE_CHOICES_HOURS: [u64; 3] = [1, 24, 168];

pub struct CleanupState {
    pub phase: CleanupPhase,
    /// Receiver of the active scan job; `None` when idle.
    pub scan_receiver: Option<Receiver<crate::infrastructure::cleanup_scan::CleanupScanMessage>>,
    /// Incremented every time a new scan starts; results carrying an older
    /// generation are discarded.
    scan_generation: Arc<AtomicU64>,
    pub scan_progress: crate::domain::cleanup::SharedCleanupScanProgress,
    /// Cancels the running scan.
    pub scan_cancel: Arc<AtomicBool>,
    /// Cancels the running delete (observed per item by the worker).
    pub delete_cancel: Arc<AtomicBool>,
    pub candidates: Vec<CleanupCandidate>,
    /// Sizes captured at scan time, used for the freed-bytes summary even
    /// when the folder was deleted (its size is no longer measurable).
    pub candidate_sizes: HashMap<PathBuf, u64>,
    pub selected: FxHashSet<PathBuf>,
    pub delete_mode: CleanupDeleteMode,
    pub age_hours: u64,
    /// Two-step confirmation for permanent deletion: first click arms, second
    /// click confirms. Cleared on reset/re-scan/mode change.
    pub permanent_confirm_armed: bool,
    /// Categories expanded in the review list.
    pub expanded_categories: FxHashSet<crate::domain::cleanup::CleanupCategoryId>,
    pub delete_progress: Option<CleanupDeleteProgress>,
    pub summary: Option<CleanupRunSummary>,
    /// Most recent runs (oldest first), capped by `CleanupRunSummary::encode_history`.
    pub history: Vec<CleanupRunSummary>,
}

impl CleanupState {
    pub fn new() -> Self {
        Self {
            phase: CleanupPhase::Idle,
            scan_receiver: None,
            scan_generation: Arc::new(AtomicU64::new(0)),
            scan_progress: Arc::new(std::sync::Mutex::new(
                crate::domain::cleanup::CleanupScanProgress::default(),
            )),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            delete_cancel: Arc::new(AtomicBool::new(false)),
            candidates: Vec::new(),
            candidate_sizes: HashMap::new(),
            selected: FxHashSet::default(),
            delete_mode: CleanupDeleteMode::RecycleBin,
            age_hours: DEFAULT_CLEANUP_AGE_HOURS,
            permanent_confirm_armed: false,
            expanded_categories: FxHashSet::default(),
            delete_progress: None,
            summary: None,
            history: Vec::new(),
        }
    }

    /// Startup constructor applying persisted preferences.
    pub fn with_persisted(
        age_hours: u64,
        delete_mode: CleanupDeleteMode,
        history: Vec<CleanupRunSummary>,
    ) -> Self {
        Self {
            age_hours,
            delete_mode,
            history,
            ..Self::new()
        }
    }

    /// Reserves a fresh scan generation and returns it.
    pub fn next_scan_generation(&self) -> u64 {
        self.scan_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn current_scan_generation(&self) -> u64 {
        self.scan_generation.load(Ordering::Relaxed)
    }

    /// True while a modal-worthy run is in flight (drives the background
    /// repaint cadence and input blocking).
    pub fn is_active(&self) -> bool {
        matches!(
            self.phase,
            CleanupPhase::Scanning | CleanupPhase::Review | CleanupPhase::Deleting
        )
    }

    /// Total bytes of the currently selected candidates.
    pub fn selected_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .filter(|candidate| self.selected.contains(&candidate.path))
            .map(|candidate| candidate.size)
            .sum()
    }

    pub fn selected_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|candidate| self.selected.contains(&candidate.path))
            .count()
    }

    /// Selects or deselects every candidate of one category.
    pub fn set_category_selected(&mut self, category: crate::domain::cleanup::CleanupCategoryId, selected: bool) {
        for candidate in &self.candidates {
            if candidate.category_id == category {
                if selected {
                    self.selected.insert(candidate.path.clone());
                } else {
                    self.selected.remove(&candidate.path);
                }
            }
        }
    }

    pub fn category_is_fully_selected(
        &self,
        category: crate::domain::cleanup::CleanupCategoryId,
    ) -> bool {
        self.candidates
            .iter()
            .filter(|candidate| candidate.category_id == category)
            .all(|candidate| self.selected.contains(&candidate.path))
    }

    pub fn category_has_candidates(
        &self,
        category: crate::domain::cleanup::CleanupCategoryId,
    ) -> bool {
        self.candidates
            .iter()
            .any(|candidate| candidate.category_id == category)
    }

    /// Resets per-run data before a new scan. Cancels any still-running
    /// previous scan first so stale workers stop spinning.
    pub fn begin_scan(&mut self) {
        self.scan_cancel.store(true, Ordering::Relaxed);
        self.scan_receiver = None;
        self.phase = CleanupPhase::Scanning;
        self.candidates.clear();
        self.candidate_sizes.clear();
        self.selected.clear();
        self.delete_progress = None;
        self.summary = None;
        self.permanent_confirm_armed = false;
        self.expanded_categories.clear();
        self.scan_cancel = Arc::new(AtomicBool::new(false));
    }

    /// Toggles the expanded state of a category section in the review list.
    pub fn toggle_category_expanded(
        &mut self,
        category: crate::domain::cleanup::CleanupCategoryId,
    ) {
        if !self.expanded_categories.remove(&category) {
            self.expanded_categories.insert(category);
        }
    }

    pub fn is_category_expanded(
        &self,
        category: crate::domain::cleanup::CleanupCategoryId,
    ) -> bool {
        self.expanded_categories.contains(&category)
    }

    /// Accepts a completed scan (already generation-checked): stores the
    /// candidates and pre-selects everything for review.
    pub fn accept_scan_result(&mut self, candidates: Vec<CleanupCandidate>) {
        self.candidate_sizes = candidates
            .iter()
            .map(|candidate| (candidate.path.clone(), candidate.size))
            .collect();
        self.selected = candidates.iter().map(|c| c.path.clone()).collect();
        self.candidates = candidates;
        self.phase = CleanupPhase::Review;
        self.scan_receiver = None;
    }

    /// Clears all per-run state (used on close/cancel/failed).
    pub fn reset(&mut self) {
        self.phase = CleanupPhase::Idle;
        self.scan_receiver = None;
        self.candidates.clear();
        self.candidate_sizes.clear();
        self.selected.clear();
        self.delete_progress = None;
        self.summary = None;
        self.permanent_confirm_armed = false;
        self.expanded_categories.clear();
    }
}

impl Default for CleanupState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cleanup::CleanupCategoryId;

    fn candidate(path: &str, category: CleanupCategoryId, size: u64) -> CleanupCandidate {
        CleanupCandidate {
            path: PathBuf::from(path),
            category_id: category,
            size,
            is_dir: false,
            modified_unix: 0,
        }
    }

    #[test]
    fn selected_bytes_and_count_track_selection() {
        let mut state = CleanupState::new();
        state.candidates = vec![
            candidate("a", CleanupCategoryId::UserTemp, 10),
            candidate("b", CleanupCategoryId::UserTemp, 5),
            candidate("c", CleanupCategoryId::CrashDumps, 100),
        ];
        state.selected.insert(PathBuf::from("a"));
        state.selected.insert(PathBuf::from("c"));
        assert_eq!(state.selected_count(), 2);
        assert_eq!(state.selected_bytes(), 110);
    }

    #[test]
    fn category_selection_toggles_only_that_category() {
        let mut state = CleanupState::new();
        state.candidates = vec![
            candidate("a", CleanupCategoryId::UserTemp, 1),
            candidate("b", CleanupCategoryId::CrashDumps, 1),
        ];
        state.set_category_selected(CleanupCategoryId::UserTemp, true);
        assert!(state.category_is_fully_selected(CleanupCategoryId::UserTemp));
        assert!(!state.category_has_candidates(CleanupCategoryId::SystemTemp));
        assert!(!state.category_is_fully_selected(CleanupCategoryId::CrashDumps));

        state.set_category_selected(CleanupCategoryId::UserTemp, false);
        assert!(!state.category_is_fully_selected(CleanupCategoryId::UserTemp));
        assert_eq!(state.selected_count(), 0);
    }

    #[test]
    fn accept_scan_result_selects_everything_and_moves_to_review() {
        let mut state = CleanupState::new();
        state.accept_scan_result(vec![candidate("a", CleanupCategoryId::UserTemp, 7)]);
        assert_eq!(state.phase, CleanupPhase::Review);
        assert_eq!(state.selected_count(), 1);
        assert_eq!(state.candidate_sizes.get(&PathBuf::from("a")), Some(&7));
        assert!(state.scan_receiver.is_none());
    }

    #[test]
    fn reset_clears_run_state() {
        let mut state = CleanupState::new();
        state.accept_scan_result(vec![candidate("a", CleanupCategoryId::UserTemp, 1)]);
        state.reset();
        assert_eq!(state.phase, CleanupPhase::Idle);
        assert!(state.candidates.is_empty());
        assert!(state.selected.is_empty());
    }

    #[test]
    fn scan_generation_increments() {
        let state = CleanupState::new();
        assert_eq!(state.next_scan_generation(), 1);
        assert_eq!(state.next_scan_generation(), 2);
        assert_eq!(state.current_scan_generation(), 2);
    }
}
