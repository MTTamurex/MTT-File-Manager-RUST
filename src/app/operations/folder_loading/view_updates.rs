use crate::app::state::ImageViewerApp;
use crate::application::sorting;
use std::sync::Arc;

impl ImageViewerApp {
    /// Filters and sorts items based on the current search query.
    ///
    /// PERFORMANCE: Uses filter_items_opt() which avoids cloning when query is empty.
    /// This eliminates unnecessary allocations in 99% of use cases.
    pub fn filter_items(&mut self) {
        let active_tag_filter = if self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            None
        } else {
            self.active_tag_filter
        };
        // PERFORMANCE: filter_items_opt returns None when query is empty,
        // signaling we should use all_items directly without cloning.
        match sorting::filter_items_opt_with_tags(
            &self.all_items,
            &self.search_query,
            active_tag_filter,
            &self.tag_assignments,
        ) {
            Some(mut filtered) => {
                sorting::sort_items(
                    &mut filtered,
                    self.sort_mode,
                    self.sort_descending,
                    self.folders_position,
                );
                self.items = Arc::new(filtered);
            }
            None => {
                // Empty query: sort all_items in-place and use directly
                // This avoids a full clone of the entire vector
                let sort_mode = self.sort_mode;
                let sort_descending = self.sort_descending;
                let folders_position = self.folders_position;
                sorting::sort_items(
                    self.all_items_mut(),
                    sort_mode,
                    sort_descending,
                    folders_position,
                );
                self.share_visible_items_from_all_items();
                self.rebuild_computer_view_indices();
            }
        }
        self.total_items = self.items.len();

        self.reconcile_visible_selection_index();
    }

    /// Sorts items based on the current mode and folder position preference.
    ///
    /// OPTIMIZED:
    /// - Uses par_sort_by for lists >5000 items (rayon)
    /// - Uses case-insensitive comparisons without allocation (natord::compare_ignore_case)
    pub fn sort_items(&mut self) {
        let active_tag_filter = if self.navigation_state.is_computer_view
            || self.navigation_state.is_recycle_bin_view
        {
            None
        } else {
            self.active_tag_filter
        };
        if self.search_query.is_empty() && active_tag_filter.is_none() {
            let sort_mode = self.sort_mode;
            let sort_descending = self.sort_descending;
            let folders_position = self.folders_position;
            sorting::sort_items(
                self.all_items_mut(),
                sort_mode,
                sort_descending,
                folders_position,
            );
            self.share_visible_items_from_all_items();
        } else {
            // PERFORMANCE: If we have unique ownership of the Arc, we can modify in-place
            // using Arc::make_mut(). Otherwise, we need to clone.
            let items = Arc::make_mut(&mut self.items);
            sorting::sort_items(
                items,
                self.sort_mode,
                self.sort_descending,
                self.folders_position,
            );
        }
        self.reconcile_visible_selection_index();
        self.rebuild_computer_view_indices();
    }

    fn rebuild_computer_view_indices(&mut self) {
        if !self.navigation_state.is_computer_view {
            return;
        }
        self.navigation_state.computer_view_local_indices.clear();
        self.navigation_state.computer_view_network_indices.clear();
        for (i, item) in self.items.iter().enumerate() {
            let is_remote = item.drive_info.as_ref().is_some_and(|di| {
                di.drive_type == crate::infrastructure::windows::DriveType::Remote
            });
            if is_remote {
                self.navigation_state.computer_view_network_indices.push(i);
            } else {
                self.navigation_state.computer_view_local_indices.push(i);
            }
        }
    }
}
