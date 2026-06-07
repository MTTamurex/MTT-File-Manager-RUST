use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn process_items_rebuild_results(&mut self, ctx: &egui::Context) {
        const MAX_REBUILD_MSGS_PER_FRAME: usize = 24;
        let rebuild_budget = if self.frame_time_peak_ms > 33.33 {
            Duration::from_millis(1)
        } else if self.frame_time_peak_ms > 25.0 {
            Duration::from_millis(2)
        } else {
            Duration::from_millis(3)
        };

        let start = Instant::now();
        let mut processed_messages = 0usize;
        let mut has_more = false;
        let mut latest_valid = None;

        while processed_messages < MAX_REBUILD_MSGS_PER_FRAME {
            if start.elapsed() >= rebuild_budget {
                has_more = true;
                break;
            }

            match self.items_rebuild_receiver.try_recv() {
                Ok(result) => {
                    processed_messages += 1;
                    if result.generation == self.generation
                        && result.request_id == self.items_rebuild_request_id
                    {
                        // Keep only the most recent valid rebuild for this frame.
                        latest_valid = Some(result);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if processed_messages >= MAX_REBUILD_MSGS_PER_FRAME {
            has_more = true;
        }

        if let Some(result) = latest_valid {
            self.items_rebuild_in_flight = false;
            self.items = Arc::new(result.items);
            self.total_items = result.total_items;
            self.hold_visible_items_until_load_complete = false;

            // After rebuild: if a pending selection was requested (e.g., after rename),
            // find the item and select + scroll to it.
            if let Some(target_path) = self.pending_select_path.take() {
                let _ = self.select_item_by_path(&target_path);
            }

            if self.pending_items_rebuild {
                self.maybe_schedule_stream_items_rebuild(ctx);
            } else {
                ctx.request_repaint();
            }
        } else if has_more {
            ctx.request_repaint();
        }
    }
}
