use crate::app::drag_drop_state::OutboundDragInputGuard;
use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::{key_state, outbound_file_drag};

use super::validation::drag_payload_inside_archive;

impl ImageViewerApp {
    /// Transfers the current internal drag to the native Windows OLE loop once
    /// the pointer leaves the client area. Returns true when a native drag was
    /// attempted and the internal drag has therefore been consumed.
    pub fn try_start_outbound_item_drag(&mut self) -> bool {
        if !self.is_item_dragging || drag_payload_inside_archive(&self.drag_payload_paths) {
            return false;
        }

        let Some(hwnd) = self.native_hwnd else {
            return false;
        };
        if !outbound_file_drag::cursor_is_outside_client(hwnd) {
            return false;
        }

        let paths = self.drag_payload_paths.clone();
        match outbound_file_drag::drag_files(&paths, hwnd) {
            Ok(outbound_file_drag::OutboundFileDragResult::ReturnedToSource) => {
                if !key_state::is_primary_mouse_button_down() {
                    self.cancel_item_drag();
                    self.arm_outbound_drag_input_guard();
                }
                self.ui_ctx.request_repaint();
                return true;
            }
            Ok(result) => {
                log::info!(
                    "[OutboundDrag] Native drag finished for {} item(s): {:?}",
                    paths.len(),
                    result
                );
            }
            Err(error) => {
                log::error!(
                    "[OutboundDrag] Native drag failed for {} item(s): {}",
                    paths.len(),
                    error
                );
                self.notifications
                    .error(rust_i18n::t!("drag_drop.outbound_error").to_string());
            }
        }

        self.cancel_item_drag();
        self.arm_outbound_drag_input_guard();
        self.ui_ctx.request_repaint();
        true
    }

    fn arm_outbound_drag_input_guard(&mut self) {
        self.outbound_drag_input_guard =
            OutboundDragInputGuard::armed(key_state::is_primary_mouse_button_down());
    }

    pub fn update_outbound_drag_input_guard(&mut self, primary_press_received_by_egui: bool) {
        let primary_down = key_state::is_primary_mouse_button_down();
        self.outbound_drag_input_guard = self
            .outbound_drag_input_guard
            .update(primary_down, primary_press_received_by_egui);
    }
}
