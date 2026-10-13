//! Cleanup feature operations
//!
//! Orchestrates a cleanup run on `ImageViewerApp`: read-only scan on a
//! worker thread, review/selection state, confirmation guards, dispatch of
//! the per-item delete through the shell operation worker, and completion
//! bookkeeping (summary, history, view refresh).

use crate::app::state::ImageViewerApp;
use crate::domain::cleanup::{
    CleanupDeleteMode, CleanupDeleteProgress, CleanupPhase, CleanupRunSummary,
};
use crate::infrastructure::cleanup_scan::{
    run_cleanup_scan, CleanupScanJob, CleanupScanMessage,
};
use rust_i18n::t;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, UNIX_EPOCH};

impl ImageViewerApp {
    /// Entry point from the context menu: opens the modal and starts a scan.
    pub fn open_cleanup_modal(&mut self) {
        if self.cleanup_state.is_active() {
            return;
        }
        self.start_cleanup_scan();
    }

    /// Closes the modal, cancelling any in-flight scan or delete.
    pub fn close_cleanup_modal(&mut self) {
        self.cancel_cleanup();
        self.cleanup_state.reset();
    }

    /// Starts a fresh read-only scan of the built-in categories.
    pub fn start_cleanup_scan(&mut self) {
        if self.cleanup_state.is_active() {
            return;
        }
        let categories = crate::infrastructure::cleanup_rules::builtin_categories();
        if categories.is_empty() {
            self.cleanup_state.phase = CleanupPhase::Failed(
                t!("cleanup.error_no_roots").to_string(),
            );
            return;
        }

        let generation = self.cleanup_state.next_scan_generation();
        self.cleanup_state.begin_scan();
        let (sender, receiver) = mpsc::channel();
        self.cleanup_state.scan_receiver = Some(receiver);

        let cutoff_unix = now_unix().saturating_sub(self.cleanup_state.age_hours * 3600);
        let cancel = Arc::clone(&self.cleanup_state.scan_cancel);
        let progress = Arc::clone(&self.cleanup_state.scan_progress);
        let spawn_result = crate::spawn_named("cleanup-scan", move || {
            run_cleanup_scan(CleanupScanJob {
                categories,
                cutoff_unix,
                cancel,
                progress,
                sender,
                generation,
            });
        });

        if spawn_result.is_err() {
            log::error!("[Cleanup] Failed to spawn scan thread");
            self.cleanup_state.scan_receiver = None;
            self.cleanup_state.phase =
                CleanupPhase::Failed(t!("cleanup.error_scan_failed").to_string());
        } else {
            self.ui_ctx.request_repaint();
        }
    }

    /// Re-scans when the age filter changes from the review UI.
    pub fn set_cleanup_age_hours(&mut self, hours: u64) {
        if self.cleanup_state.age_hours == hours {
            return;
        }
        self.cleanup_state.age_hours = hours;
        self.save_preferences();
        self.start_cleanup_scan();
    }

    pub fn set_cleanup_delete_mode(&mut self, mode: CleanupDeleteMode) {
        if self.cleanup_state.delete_mode == mode {
            return;
        }
        self.cleanup_state.delete_mode = mode;
        self.save_preferences();
    }

    /// Cancels the in-flight scan or delete (the delete finishes its current
    /// item, then stops).
    pub fn cancel_cleanup(&mut self) {
        match self.cleanup_state.phase {
            CleanupPhase::Scanning => {
                self.cleanup_state.scan_cancel.store(true, Ordering::Relaxed);
            }
            CleanupPhase::Deleting => {
                self.cleanup_state
                    .delete_cancel
                    .store(true, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// Confirms the reviewed selection and dispatches the delete run.
    pub fn confirm_cleanup(&mut self) {
        if self.cleanup_state.phase != CleanupPhase::Review {
            return;
        }
        let selected: Vec<PathBuf> = self
            .cleanup_state
            .candidates
            .iter()
            .filter(|candidate| self.cleanup_state.selected.contains(&candidate.path))
            .map(|candidate| candidate.path.clone())
            .collect();
        if selected.is_empty() {
            return;
        }

        // Guard: never delete anything that is (an ancestor of) a folder
        // currently open in a panel.
        if selected
            .iter()
            .any(|path| self.path_is_same_or_ancestor_of_open_panel(path))
        {
            self.notifications
                .warning(t!("cleanup.guard_open_panel").to_string());
            return;
        }

        let permanent = self.cleanup_state.delete_mode == CleanupDeleteMode::Permanent;
        // The cutoff captured at scan time is reused so the worker's stale
        // rejection matches what the user reviewed.
        let cutoff_unix = now_unix().saturating_sub(self.cleanup_state.age_hours * 3600);
        let cancel = Arc::clone(&self.cleanup_state.delete_cancel);

        self.cleanup_state.phase = CleanupPhase::Deleting;
        self.cleanup_state.delete_progress = Some(CleanupDeleteProgress {
            completed: 0,
            total: selected.len(),
            current_name: None,
            failed_count: 0,
        });

        self.file_operation_state.file_ops_in_progress += 1;
        let request = crate::workers::file_operation_worker::FileOperationRequest::cleanup_delete(
            selected,
            permanent,
            cutoff_unix,
            cancel,
            self.shell_op_hwnd(),
        );
        if self.file_operation_state.file_op_sender.send(request).is_err() {
            self.file_operation_state.file_ops_in_progress = self
                .file_operation_state
                .file_ops_in_progress
                .saturating_sub(1);
            log::warn!("[Cleanup] Worker channel closed on cleanup delete");
            self.cleanup_state.phase =
                CleanupPhase::Failed(t!("cleanup.error_scan_failed").to_string());
        }
    }

    /// Drains scan-worker messages (called once per frame).
    pub fn poll_cleanup_updates(&mut self) {
        let receiver = match self.cleanup_state.scan_receiver.take() {
            Some(receiver) => receiver,
            None => return,
        };
        loop {
            match receiver.try_recv() {
                Ok(CleanupScanMessage::Completed {
                    generation,
                    candidates,
                }) => {
                    if generation == self.cleanup_state.current_scan_generation()
                        && self.cleanup_state.phase == CleanupPhase::Scanning
                    {
                        self.cleanup_state.accept_scan_result(candidates);
                        self.ui_ctx.request_repaint();
                        return;
                    }
                    // Stale result from a superseded scan: keep draining.
                }
                Ok(CleanupScanMessage::Failed { generation, error }) => {
                    if generation == self.cleanup_state.current_scan_generation()
                        && self.cleanup_state.phase == CleanupPhase::Scanning
                    {
                        self.cleanup_state.phase = CleanupPhase::Failed(error);
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.cleanup_state.scan_receiver = Some(receiver);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Scan thread died without a message (panic): fail the
                    // phase instead of spinning in Scanning forever.
                    if self.cleanup_state.phase == CleanupPhase::Scanning {
                        self.cleanup_state.phase =
                            CleanupPhase::Failed(t!("cleanup.error_scan_failed").to_string());
                    }
                    return;
                }
            }
        }
    }

    /// Per-item progress from the delete worker (via the file-op result
    /// channel).
    pub fn handle_cleanup_delete_progress(
        &mut self,
        completed: usize,
        total: usize,
        current_name: Option<String>,
        failed_count: usize,
    ) {
        if self.cleanup_state.phase != CleanupPhase::Deleting {
            return;
        }
        self.cleanup_state.delete_progress = Some(CleanupDeleteProgress {
            completed,
            total,
            current_name,
            failed_count,
        });
    }

    /// Final result of the delete run (via the file-op result channel).
    pub fn handle_cleanup_delete_completed(
        &mut self,
        deleted_paths: Vec<PathBuf>,
        failed: Vec<(PathBuf, String)>,
        aborted: bool,
    ) {
        // The worker reports completion itself (NoFinished), so the
        // file_ops_in_progress accounting is owned here.
        self.file_operation_state.file_ops_in_progress = self
            .file_operation_state
            .file_ops_in_progress
            .saturating_sub(1);

        if self.cleanup_state.phase != CleanupPhase::Deleting {
            return; // stale or superseded run
        }

        let deleted_bytes = CleanupRunSummary::deleted_bytes_from(
            &deleted_paths,
            &self.cleanup_state.candidate_sizes,
        );
        let summary = CleanupRunSummary {
            deleted_count: deleted_paths.len(),
            deleted_bytes,
            skipped: failed
                .iter()
                .map(|(path, reason)| (path.to_string_lossy().into_owned(), reason.clone()))
                .collect(),
            aborted,
            permanent: self.cleanup_state.delete_mode == CleanupDeleteMode::Permanent,
            finished_at_unix: now_unix(),
        };
        self.cleanup_state.history.push(summary.clone());
        self.save_preferences();
        self.cleanup_state.summary = Some(summary.clone());
        self.cleanup_state.phase = CleanupPhase::Completed;
        self.cleanup_state.delete_progress = None;

        self.refresh_views_after_cleanup(&deleted_paths);

        let freed = crate::infrastructure::windows::formatting::format_size(summary.deleted_bytes);
        if aborted {
            self.notifications.warning(t!(
                "cleanup.done_aborted",
                freed = freed,
                deleted = summary.deleted_count
            ).to_string());
        } else {
            self.notifications.success(t!(
                "cleanup.done",
                freed = freed,
                deleted = summary.deleted_count
            ).to_string());
        }
        log::info!(
            "[Cleanup] Run finished: deleted={} freed={freed} skipped={} aborted={aborted} permanent={}",
            summary.deleted_count,
            summary.skipped.len(),
            summary.permanent,
        );
    }

    /// Reloads open views that displayed a folder containing deleted items.
    fn refresh_views_after_cleanup(&mut self, deleted_paths: &[PathBuf]) {
        if deleted_paths.is_empty() {
            return;
        }
        let current = PathBuf::from(&self.navigation_state.current_path);
        let current_affected = deleted_paths
            .iter()
            .any(|path| path.parent() == Some(&current));
        // Suppress watcher reload storms triggered by the shell deletes.
        self.watcher_cooldown_until = Some(Instant::now() + Duration::from_secs(2));
        if current_affected {
            self.loaded_path.clear();
            self.reload_current_folder_preserving_icon_cache();
        }
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
