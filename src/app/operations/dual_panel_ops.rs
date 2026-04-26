//! Dual panel operations: enable, disable, switch active panel, swap state.

use crate::app::dual_panel::{ActivePanel, PanelSnapshot};
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// Enable dual panel mode.
    ///
    /// The current app state becomes the **left** panel. The right panel is
    /// initialised as a clone of the current state (same path).
    pub fn dual_panel_enable(&mut self) {
        if self.dual_panel_enabled {
            return;
        }
        log::info!("[DualPanel] Enabling dual panel mode");

        // Current state becomes left panel (stays in app fields).
        // Right panel starts as a snapshot of the current state.
        self.dual_panel_active = ActivePanel::Left;
        let mut snapshot = PanelSnapshot::from_app(self);
        // Keep the SAME current_generation Arc so thumbnail workers stay
        // synchronized with whichever panel is active.  Each panel has its
        // own generation value for routing folder-load results, but the
        // shared gen_tracker must match the active panel's generation.
        snapshot.is_loading_folder = false;
        snapshot.pending_all_items_clear = false;
        snapshot.pending_items_rebuild = false;
        snapshot.pending_items_count = 0;
        self.dual_panel_inactive_state = Some(snapshot);
        self.dual_panel_enabled = true;
    }

    /// Disable dual panel mode.
    ///
    /// The active panel's state remains in the app fields; the inactive panel
    /// is discarded.
    pub fn dual_panel_disable(&mut self) {
        if !self.dual_panel_enabled {
            return;
        }
        log::info!("[DualPanel] Disabling dual panel mode");

        // Drop inactive panel state
        self.dual_panel_inactive_state = None;
        self.dual_panel_enabled = false;
    }

    /// Toggle dual panel mode on/off.
    pub fn dual_panel_toggle(&mut self) {
        if self.dual_panel_enabled {
            self.dual_panel_disable();
        } else {
            self.dual_panel_enable();
        }
    }

    /// Switch the active panel (Left ↔ Right).
    ///
    /// Uses zero-allocation `swap_with_app` to exchange state between the
    /// active panel (in app fields) and the inactive panel (in snapshot).
    pub fn dual_panel_switch_active(&mut self) {
        if !self.dual_panel_enabled {
            return;
        }
        let Some(mut snapshot) = self.dual_panel_inactive_state.take() else {
            log::warn!("[DualPanel] switch_active called but no inactive state");
            return;
        };

        log::debug!(
            "[DualPanel] Switching active panel from {:?} to {:?}",
            self.dual_panel_active,
            self.dual_panel_active.other()
        );

        // Zero-alloc swap: active ↔ inactive
        snapshot.swap_with_app(self);
        self.dual_panel_inactive_state = Some(snapshot);
        self.dual_panel_active = self.dual_panel_active.other();

        // Sync the shared gen tracker with the newly active panel's generation
        // so thumbnail workers accept requests from the now-active panel.
        self.current_generation
            .store(self.generation, std::sync::atomic::Ordering::Relaxed);

        // Re-watch the new active folder so watcher events go to the right place
        self.watch_current_folder();

        // Check if the newly active panel's folder was marked dirty while inactive
        // (e.g., by a file operation that completed while this panel was inactive).
        let current = std::path::PathBuf::from(&self.navigation_state.current_path);
        if self.directory_dirty_registry.is_dirty(&current) {
            log::info!(
                "[DualPanel] Newly active panel folder is dirty, reloading: {:?}",
                current
            );
            self.loaded_path.clear();
            self.load_folder(false);
        }
    }

    /// Temporarily swap the inactive panel into app fields, run a closure, then
    /// swap back. Used for rendering the inactive panel and for triggering
    /// async folder loads on the inactive panel.
    ///
    /// Uses zero-allocation `swap_with_app` twice (swap in, run, swap out).
    ///
    /// **Safe for `load_folder`** since each panel has its own `generation` /
    /// `current_generation`; results are routed in `process_streaming_and_thumbnail_events`.
    pub fn with_inactive_panel<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        if !self.dual_panel_enabled {
            return None;
        }
        // Take the snapshot out so we can get a mutable ref
        let mut snapshot = self.dual_panel_inactive_state.take()?;

        // Swap inactive state INTO app fields (active state goes into snapshot)
        snapshot.swap_with_app(self);

        // Run the closure with inactive state now in app fields
        let result = f(self);

        // Swap back: restore active state into app fields
        snapshot.swap_with_app(self);

        // Put snapshot back
        self.dual_panel_inactive_state = Some(snapshot);

        Some(result)
    }

    /// Get the path of the inactive panel (used for cross-panel operations).
    pub fn dual_panel_inactive_path(&self) -> Option<&str> {
        self.dual_panel_inactive_state
            .as_ref()
            .map(|s| s.path.as_str())
    }

    /// Copy selected items from the active panel to the inactive panel's folder.
    pub fn dual_panel_copy_to_other(&mut self) {
        if !self.dual_panel_enabled {
            return;
        }
        let Some(dest_path) = self.dual_panel_inactive_state.as_ref().map(|s| s.path.clone()) else {
            return;
        };
        let dest = std::path::PathBuf::from(&dest_path);

        // Gather selected paths
        let sources = self.gather_selected_paths();
        if sources.is_empty() {
            return;
        }

        log::info!(
            "[DualPanel] Copy {} items to other panel: {:?}",
            sources.len(),
            dest
        );

        let hwnd = self.native_hwnd.unwrap_or_default();
        let req = crate::workers::file_operation_worker::FileOperationRequest::copy_batch(
            sources,
            dest,
            hwnd,
        );
        self.file_operation_state.file_ops_in_progress += 1;
        if self.file_operation_state.file_op_sender.send(req).is_err() {
            self.file_operation_state.file_ops_in_progress =
                self.file_operation_state.file_ops_in_progress.saturating_sub(1);
            log::warn!("[DualPanel] worker channel closed on cross-panel copy");
        }
    }

    /// Move selected items from the active panel to the inactive panel's folder.
    pub fn dual_panel_move_to_other(&mut self) {
        if !self.dual_panel_enabled {
            return;
        }
        let Some(dest_path) = self.dual_panel_inactive_state.as_ref().map(|s| s.path.clone()) else {
            return;
        };
        let dest = std::path::PathBuf::from(&dest_path);

        let sources = self.gather_selected_paths();
        if sources.is_empty() {
            return;
        }

        log::info!(
            "[DualPanel] Move {} items to other panel: {:?}",
            sources.len(),
            dest
        );

        let hwnd = self.native_hwnd.unwrap_or_default();
        let req = crate::workers::file_operation_worker::FileOperationRequest::move_batch(
            sources,
            dest,
            hwnd,
        );
        self.file_operation_state.file_ops_in_progress += 1;
        if self.file_operation_state.file_op_sender.send(req).is_err() {
            self.file_operation_state.file_ops_in_progress =
                self.file_operation_state.file_ops_in_progress.saturating_sub(1);
            log::warn!("[DualPanel] worker channel closed on cross-panel move");
        }
    }

    /// Gather selected file paths from the active panel (multi-selection or single).
    fn gather_selected_paths(&self) -> Vec<std::path::PathBuf> {
        if !self.multi_selection.is_empty() {
            self.multi_selection.iter().cloned().collect()
        } else if let Some(idx) = self.selected_item {
            if let Some(item) = self.items.get(idx) {
                vec![item.path.clone()]
            } else {
                vec![]
            }
        } else {
            vec![]
        }
    }
}
