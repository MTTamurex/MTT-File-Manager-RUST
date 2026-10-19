//! Keep size sorting coherent with asynchronous listings and both visible panes.
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::SortMode;
use std::time::{Duration, Instant};

impl ImageViewerApp {
    pub(super) fn prepare_folder_size_sort(&mut self, force: bool) -> bool {
        // Temporary inactive-panel swaps must not replace the active pane's
        // tracked folders. The regular update prepares the union of both panes.
        if self.in_inactive_panel_context {
            return false;
        }
        let active = self.sort_mode == SortMode::Size && !self.navigation_state.is_computer_view;
        let other = self.dual_panel_inactive_state.as_ref().filter(|panel| {
            self.dual_panel_enabled && panel.sort_mode == SortMode::Size && !panel.is_computer_view
        });
        if !active && other.is_none() {
            return false;
        }
        let changed = self.folder_size_state.prepare_size_sort_for_listings(
            if active {
                self.all_items.as_slice()
            } else {
                &[]
            },
            other.map_or(&[], |panel| panel.all_items.as_slice()),
            Instant::now(),
            force,
        );
        if changed {
            self.folder_size_state.size_sorting.forget_sorted_listings();
        }
        changed
    }

    pub(in crate::app) fn folder_size_sorting_needs_update(&self) -> bool {
        let active = self.sort_mode == SortMode::Size && !self.navigation_state.is_computer_view;
        let other = self.dual_panel_inactive_state.as_ref().filter(|panel| {
            self.dual_panel_enabled && panel.sort_mode == SortMode::Size && !panel.is_computer_view
        });
        ((active || other.is_some()) && self.folder_size_state.size_sorting.needs_update())
            || (active && !self.folder_size_state.size_sorting.is_sorted(&self.items))
            || other
                .is_some_and(|panel| !self.folder_size_state.size_sorting.is_sorted(&panel.items))
    }

    pub(super) fn remember_folder_size_sort(&mut self) {
        if self.sort_mode == SortMode::Size && !self.navigation_state.is_computer_view {
            self.folder_size_state
                .size_sorting
                .remember_sorted(&self.items);
            if self.folder_size_state.size_sorting.needs_update() {
                self.ui_ctx
                    .request_repaint_after(Duration::from_millis(100));
            }
        }
    }

    pub(in crate::app) fn update_folder_size_sorting(&mut self) -> bool {
        let active = self.sort_mode == SortMode::Size && !self.navigation_state.is_computer_view;
        let inactive = self.dual_panel_enabled
            && self
                .dual_panel_inactive_state
                .as_ref()
                .is_some_and(|panel| panel.sort_mode == SortMode::Size && !panel.is_computer_view);
        if !active && !inactive {
            return false;
        }

        let changed = self.prepare_folder_size_sort(false);
        let mut sorted = false;
        if active
            && !self.hold_visible_items_until_load_complete
            && (changed || !self.folder_size_state.size_sorting.is_sorted(&self.items))
        {
            self.sort_items();
            sorted = true;
        }
        let other_unsorted = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|panel| !self.folder_size_state.size_sorting.is_sorted(&panel.items));
        let other_holds_listing = self
            .dual_panel_inactive_state
            .as_ref()
            .is_some_and(|panel| panel.hold_visible_items_until_load_complete);
        if inactive && !other_holds_listing && (changed || other_unsorted) {
            self.with_inactive_panel(|app| app.sort_items());
            sorted = true;
        }
        if self.folder_size_state.size_sorting.needs_update() {
            self.ui_ctx
                .request_repaint_after(Duration::from_millis(100));
        }
        sorted
    }
}
