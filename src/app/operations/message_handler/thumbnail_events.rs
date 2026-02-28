use crate::app::state::ImageViewerApp;
use eframe::egui;
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

        // SAFETY TIMEOUT: Clear is_loading_folder if stuck for more than 30 seconds
        // This prevents infinite spinner if the loading thread fails silently
        if self.is_loading_folder && self.loading_started_at.elapsed().as_secs() > 30 {
            log::warn!(
                "[FOLDER-LOADING] TIMEOUT: Loading took more than 30 seconds, clearing loading state"
            );
            self.is_loading_folder = false;
        }

        let mut saw_end_of_load = false;
        let mut processed_batches = 0usize;
        let mut has_more_stream_batches = false;
        let stream_start = Instant::now();
        while processed_batches < MAX_FILE_BATCHES_PER_FRAME {
            if stream_start.elapsed() >= std::time::Duration::from_millis(FILE_BATCH_BUDGET_MS) {
                has_more_stream_batches = true;
                break;
            }
            match self.file_entry_receiver.try_recv() {
                Ok((gen_id, new_batch)) => {
                    processed_batches += 1;
                    if gen_id != self.generation {
                        continue; // Discard data from a previous navigation/refresh
                    }

                    if new_batch.is_empty() {
                        // Empty batch = "End of Loading" signal from thread
                        saw_end_of_load = true;
                    } else {
                        // Deferred clear: replace stale items on first real batch
                        // so the UI never shows an empty list during watcher reloads.
                        if self.pending_all_items_clear {
                            self.all_items.clear();
                            self.pending_all_items_clear = false;
                        }
                        // Data arrived! Add to master list
                        self.pending_items_count =
                            self.pending_items_count.saturating_add(new_batch.len());
                        self.pending_items_rebuild = true;
                        self.all_items.extend(new_batch);
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break, // No more messages
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        if processed_batches >= MAX_FILE_BATCHES_PER_FRAME {
            has_more_stream_batches = true;
        }

        let t_stream_recv = Instant::now();

        if saw_end_of_load {
            self.handle_items_after_end_of_load(ctx);
        } else if self.pending_items_rebuild {
            self.maybe_schedule_stream_items_rebuild(ctx);
        } else if has_more_stream_batches {
            ctx.request_repaint();
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
}
