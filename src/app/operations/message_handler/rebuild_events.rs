use crate::app::state::ImageViewerApp;
use eframe::egui;
use std::sync::mpsc::TryRecvError;
use std::sync::Arc;

impl ImageViewerApp {
    pub(super) fn process_items_rebuild_results(&mut self, ctx: &egui::Context) {
        loop {
            match self.items_rebuild_receiver.try_recv() {
                Ok(result) => {
                    if result.generation != self.generation {
                        continue;
                    }
                    if result.request_id != self.items_rebuild_request_id {
                        continue;
                    }
                    self.items = Arc::new(result.items);
                    self.total_items = result.total_items;

                    // After rebuild: if a pending selection was requested (e.g., after rename),
                    // find the item and select + scroll to it.
                    if let Some(target_path) = self.pending_select_path.take() {
                        if let Some(idx) = self.items.iter().position(|i| i.path == target_path) {
                            self.selected_item = Some(idx);
                            self.selected_file = Some(self.items[idx].clone());
                            self.scroll_to_selected = true;
                        }
                    }

                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
    }
}
