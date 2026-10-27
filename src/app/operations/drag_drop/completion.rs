use super::validation::{should_confirm_drag_move, DragDropOperation};
use crate::app::drag_drop_state::{ItemDragOrigin, PendingDragMoveConfirmation};
use crate::app::state::ImageViewerApp;
use crate::workers::file_operation_worker::FileOperationRequest;

impl ImageViewerApp {
    /// Uses the modifiers held at release, not those held when the drag started.
    pub fn complete_item_drag(&mut self, ctrl_pressed: bool, shift_pressed: bool) {
        if self.file_panel_input_blocked_by_drag_move_confirmation() {
            self.cancel_item_drag();
            return;
        }
        if !self.is_item_dragging {
            return;
        }
        let drag_origin = self.item_drag_origin;
        let cross_panel_target = self.drag_cross_panel_target.clone().filter(|target| {
            !target
                .to_str()
                .is_some_and(crate::domain::special_paths::is_virtual_path)
        });
        let Some(dest_folder) = self.drag_target_folder.clone().or(cross_panel_target) else {
            self.cancel_item_drag();
            return;
        };
        let dest_folder = self.resolve_network_location_path(&dest_folder);
        if !self.is_valid_drop_target(&dest_folder) {
            self.cancel_item_drag();
            return;
        }
        let operation = self.resolve_drag_operation(&dest_folder, ctrl_pressed, shift_pressed);
        let Some(source_folder) = self.drag_source_folder.clone() else {
            self.cancel_item_drag();
            return;
        };
        let paths = std::mem::take(&mut self.drag_payload_paths);
        if paths.is_empty() {
            self.cancel_item_drag();
            return;
        }
        if should_confirm_drag_move(operation) {
            self.pending_drag_move_confirmation = Some(PendingDragMoveConfirmation::new(
                paths,
                dest_folder,
                Some(source_folder),
                drag_origin,
            ));
            self.cancel_item_drag();
            if drag_origin == ItemDragOrigin::GlobalSearch {
                self.global_search.suspended_for_drag = true;
            }
            self.ui_ctx.request_repaint();
            return;
        }
        let hwnd = self.shell_op_hwnd();
        let request = match operation {
            DragDropOperation::Copy => FileOperationRequest::copy_batch(paths, dest_folder, hwnd),
            DragDropOperation::Move => FileOperationRequest::move_batch(paths, dest_folder, hwnd),
            DragDropOperation::Link => FileOperationRequest::CreateShortcuts { paths, dest_folder },
        };
        if !self.dispatch_drag_file_operation(request) {
            self.cancel_item_drag();
            self.ui_ctx.request_repaint();
            return;
        }
        if drag_origin == ItemDragOrigin::GlobalSearch {
            self.cancel_item_drag();
            self.close_global_search();
        } else {
            self.clear_selection_after_drag_file_operation(Some(&source_folder));
        }
        self.ui_ctx.request_repaint();
    }
}
