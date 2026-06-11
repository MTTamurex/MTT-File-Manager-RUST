use crate::app::state::{FolderLoadError, FolderLoadErrorKind, ImageViewerApp};
use crate::domain::file_entry::FileEntry;
use eframe::egui;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn process_streaming_and_thumbnail_events(
        &mut self,
        ctx: &egui::Context,
    ) -> Instant {
        const MAX_FILE_BATCHES_PER_FRAME: usize = 48;
        const FILE_BATCH_BUDGET_MS: u64 = 5;

        // 1. STREAMING: Receive incremental batches of FileEntry (filtered by generation)
        // BLOCKING: Process all available file entries in batch

        if self.process_folder_load_failures(ctx) {
            return Instant::now();
        }

        // SAFETY TIMEOUT: Clear is_loading_folder if stuck for more than 30 seconds
        // This prevents infinite spinner if the loading thread fails silently
        if self.is_loading_folder && self.loading_started_at.elapsed().as_secs() > 30 {
            log::warn!(
                "[FOLDER-LOADING] TIMEOUT: Loading took more than 30 seconds, clearing loading state"
            );
            self.is_loading_folder = false;
            self.hold_visible_items_until_load_complete = false;
            self.file_operation_state.pending_deletions.clear();
        }

        // Also check inactive panel loading timeout
        if self.dual_panel_enabled {
            if let Some(ref mut snapshot) = self.dual_panel_inactive_state {
                if snapshot.is_loading_folder
                    && snapshot.loading_started_at.elapsed().as_secs() > 30
                {
                    log::warn!("[FOLDER-LOADING] TIMEOUT: Inactive panel loading timed out");
                    snapshot.is_loading_folder = false;
                    snapshot.hold_visible_items_until_load_complete = false;
                }
            }
        }

        // Determine inactive panel generation for routing (if dual panel active)
        let inactive_gen: Option<usize> = if self.dual_panel_enabled {
            self.dual_panel_inactive_state
                .as_ref()
                .map(|s| s.generation)
        } else {
            None
        };

        let mut saw_end_of_load = false;
        let mut processed_batches = 0usize;
        let mut has_more_stream_batches = false;
        let stream_start = Instant::now();

        // Collect batches destined for the inactive panel (processed after the loop)
        let mut inactive_batches: Vec<Vec<FileEntry>> = Vec::new();
        let mut inactive_saw_end = false;

        while processed_batches < MAX_FILE_BATCHES_PER_FRAME {
            if stream_start.elapsed() >= std::time::Duration::from_millis(FILE_BATCH_BUDGET_MS) {
                has_more_stream_batches = true;
                break;
            }
            match self.file_entry_receiver.try_recv() {
                Ok((gen_id, new_batch)) => {
                    processed_batches += 1;

                    if gen_id == self.generation {
                        // ── Active panel batch ──
                        if new_batch.is_empty() {
                            saw_end_of_load = true;
                        } else {
                            if self.pending_all_items_clear {
                                self.capture_stale_items_snapshot();
                                self.all_items_mut().clear();
                                self.pending_all_items_clear = false;
                            }
                            self.pending_items_count =
                                self.pending_items_count.saturating_add(new_batch.len());
                            self.pending_items_rebuild = true;
                            self.all_items_mut().extend(new_batch);
                        }
                    } else if let Some(ig) = inactive_gen {
                        if gen_id == ig {
                            // ── Inactive panel batch ──
                            if new_batch.is_empty() {
                                inactive_saw_end = true;
                            } else {
                                inactive_batches.push(new_batch);
                            }
                        }
                        // else: stale generation, discard
                    }
                    // else: stale generation, discard
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        if processed_batches >= MAX_FILE_BATCHES_PER_FRAME {
            has_more_stream_batches = true;
        }

        let t_stream_recv = Instant::now();

        // Process active panel end-of-load / rebuild
        if saw_end_of_load {
            self.handle_items_after_end_of_load(ctx);
        } else if self.pending_items_rebuild {
            self.maybe_schedule_stream_items_rebuild(ctx);
        } else if has_more_stream_batches {
            ctx.request_repaint();
        }

        // Process inactive panel batches via with_inactive_panel
        if !inactive_batches.is_empty() || inactive_saw_end {
            self.apply_inactive_panel_batches(inactive_batches, inactive_saw_end, ctx);
        } else {
            self.maybe_rebuild_inactive_panel_items(ctx);
        }

        let t_rebuild = Instant::now();

        // 2. Cover worker results
        self.process_cover_worker_results(ctx);

        let streaming_done = Instant::now();

        // 3. Icon worker results
        self.process_icon_worker_results(ctx);
        let t_icons = Instant::now();

        // 4. Thumbnails + folder previews upload pipeline (higher UX priority)
        let mut received_any = self.process_thumbnail_upload_pipeline(ctx);
        let t_thumbs = Instant::now();

        // Under frame pressure, preserve smoothness by deferring lower-priority
        // metadata/folder-size processing to subsequent frames.
        let pressure_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(10)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(12)
        } else {
            Duration::from_millis(16)
        };
        let should_defer_low_priority = stream_start.elapsed() >= pressure_budget;

        // 5. Metadata worker results (lower priority than visible media uploads)
        if !should_defer_low_priority {
            self.process_metadata_worker_results(ctx);
            self.process_live_file_size_worker_results(ctx);
        }
        let t_meta = Instant::now();

        // 6. Folder size updates (lowest priority in critical frames)
        if !should_defer_low_priority {
            received_any |= self.process_folder_size_results();
        }
        let t_sizes = Instant::now();

        let total_ms = stream_start.elapsed().as_millis();
        if total_ms > 50 {
            log::warn!(
                "[PERF-STREAM] recv={}ms rebuild={}ms covers={}ms | icons={}ms meta={}ms thumbs={}ms sizes={}ms (batches={} items={} eol={})",
                t_stream_recv.duration_since(stream_start).as_millis(),
                t_rebuild.duration_since(t_stream_recv).as_millis(),
                streaming_done.duration_since(t_rebuild).as_millis(),
                t_icons.duration_since(streaming_done).as_millis(),
                t_meta.duration_since(t_thumbs).as_millis(),
                t_thumbs.duration_since(t_icons).as_millis(),
                t_sizes.duration_since(t_thumbs).as_millis(),
                processed_batches,
                self.pending_items_count,
                saw_end_of_load,
            );
        }

        if received_any || has_more_stream_batches {
            ctx.request_repaint();
        }

        t_sizes
    }

    fn apply_folder_load_error_to_current_panel(&mut self, failure: FolderLoadError) {
        log::warn!(
            "[FOLDER-LOADING] Current folder load failed: kind={:?} path={} message={:?}",
            failure.kind,
            failure.path.display(),
            failure.message
        );

        self.is_loading_folder = false;
        self.pending_all_items_clear = false;
        self.hold_visible_items_until_load_complete = false;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.items = Arc::new(Vec::new());
        self.all_items_mut().clear();
        self.total_items = 0;
        self.selected_item = None;
        self.selected_file = None;
        self.folder_load_error = Some(failure);
    }

    fn process_folder_load_failures(&mut self, ctx: &egui::Context) -> bool {
        let mut handled_current = false;

        while let Ok((gen_id, failure)) = self.folder_load_failure_receiver.try_recv() {
            if gen_id != self.generation {
                let inactive_gen = self
                    .dual_panel_inactive_state
                    .as_ref()
                    .map(|snapshot| snapshot.generation);
                if !self.dual_panel_enabled || Some(gen_id) != inactive_gen {
                    continue;
                }

                let mut handled_inactive = false;
                self.with_inactive_panel(|app| {
                    let current_path = std::path::PathBuf::from(&app.navigation_state.current_path);
                    if Self::normalize_for_match(&failure.path)
                        == Self::normalize_for_match(&current_path)
                    {
                        app.apply_folder_load_error_to_current_panel(failure.clone());
                        handled_inactive = true;
                    }
                });

                if handled_inactive {
                    ctx.request_repaint();
                    handled_current = true;
                }
                continue;
            }

            let current_path = std::path::PathBuf::from(&self.navigation_state.current_path);
            if Self::normalize_for_match(&failure.path) != Self::normalize_for_match(&current_path)
            {
                continue;
            }

            match failure.kind {
                FolderLoadErrorKind::NotFound => {
                    log::warn!(
                        "[FOLDER-LOADING] Current folder load failed because path vanished: {:?}",
                        failure.path
                    );
                    self.is_loading_folder = false;
                    self.pending_all_items_clear = false;
                    self.hold_visible_items_until_load_complete = false;
                    self.pending_auto_reload = false;
                    self.skip_next_auto_reload = false;
                    self.folder_load_error = None;
                    self.navigate_to_nearest_valid_ancestor();
                }
                FolderLoadErrorKind::AccessDenied | FolderLoadErrorKind::Other => {
                    self.pending_auto_reload = false;
                    self.skip_next_auto_reload = false;
                    self.apply_folder_load_error_to_current_panel(failure);
                }
            }
            ctx.request_repaint();
            handled_current = true;
        }

        handled_current
    }

    /// Apply collected batches to the inactive panel via state swap.
    /// Avoids calling `handle_items_after_end_of_load` directly (which has
    /// global side-effects); instead performs lightweight filter + sort.
    fn apply_inactive_panel_batches(
        &mut self,
        batches: Vec<Vec<FileEntry>>,
        saw_end: bool,
        ctx: &egui::Context,
    ) {
        let ctx2 = ctx.clone();
        self.with_inactive_panel(|app| {
            for batch in batches {
                if app.pending_all_items_clear {
                    app.capture_stale_items_snapshot();
                    app.all_items_mut().clear();
                    app.pending_all_items_clear = false;
                }
                app.pending_items_count = app.pending_items_count.saturating_add(batch.len());
                app.pending_items_rebuild = true;
                app.all_items_mut().extend(batch);
            }

            if saw_end {
                app.is_loading_folder = false;
                if app.pending_all_items_clear {
                    app.all_items_mut().clear();
                    app.pending_all_items_clear = false;
                }
                // Reconcile stale textures
                app.reconcile_stale_visual_caches();
                app.pending_items_rebuild = false;
                app.pending_items_count = 0;
                app.filter_items();
                app.hold_visible_items_until_load_complete = false;
                app.loaded_path = app.navigation_state.current_path.clone();
                log::info!(
                    "[DualPanel] Inactive panel reload complete: {}",
                    app.navigation_state.current_path
                );
            }
        });
        self.maybe_rebuild_inactive_panel_items(&ctx2);
        ctx2.request_repaint();
    }

    fn maybe_rebuild_inactive_panel_items(&mut self, ctx: &egui::Context) {
        if !self.dual_panel_enabled {
            return;
        }

        let ctx2 = ctx.clone();
        let mut rebuilt = false;
        self.with_inactive_panel(|app| {
            if !app.pending_items_rebuild {
                return;
            }

            if app.hold_visible_items_until_load_complete && app.is_loading_folder {
                return;
            }

            // UX: the inactive dual-panel is still visible, so its listing must
            // reflect watcher/file-op changes as soon as the next frame arrives.
            // Keep the batch coalescing done earlier in this frame, but do not
            // throttle the visual rebuild behind the active-panel debounce.
            app.filter_items();
            app.pending_items_rebuild = false;
            app.pending_items_count = 0;
            app.last_items_rebuild = Instant::now();
            rebuilt = true;
        });

        if rebuilt {
            ctx2.request_repaint();
        }
    }
}
