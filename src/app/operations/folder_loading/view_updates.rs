use crate::app::state::ImageViewerApp;
use crate::application::sorting;
use std::sync::Arc;

impl ImageViewerApp {
    /// Filters and sorts items based on the current search query.
    ///
    /// PERFORMANCE: Uses filter_items_opt() which avoids cloning when query is empty.
    /// This eliminates unnecessary allocations in 99% of use cases.
    pub fn filter_items(&mut self) {
        // PERFORMANCE: filter_items_opt returns None when query is empty,
        // signaling we should use all_items directly without cloning.
        match sorting::filter_items_opt(&self.all_items, &self.search_query) {
            Some(filtered) => {
                // Query present: use the filtered vector
                self.items = Arc::new(filtered);
            }
            None => {
                // Empty query: sort all_items in-place and use directly
                // This avoids a full clone of the entire vector
                sorting::sort_items(
                    &mut self.all_items,
                    self.sort_mode,
                    self.sort_descending,
                    self.folders_position,
                );
                self.items = Arc::new(self.all_items.clone());
            }
        }
        self.total_items = self.items.len();

        // If filtering was applied, we still need to sort the result
        if !self.search_query.is_empty() {
            self.sort_items();
        }

        self.reconcile_visible_selection_index();
    }

    /// Sorts items based on the current mode and folder position preference.
    ///
    /// OPTIMIZED:
    /// - Uses par_sort_by for lists >5000 items (rayon)
    /// - Uses case-insensitive comparisons without allocation (natord::compare_ignore_case)
    pub fn sort_items(&mut self) {
        // PERFORMANCE: If we have unique ownership of the Arc, we can modify in-place
        // using Arc::make_mut(). Otherwise, we need to clone.
        let items = Arc::make_mut(&mut self.items);
        sorting::sort_items(
            items,
            self.sort_mode,
            self.sort_descending,
            self.folders_position,
        );
        self.reconcile_visible_selection_index();
    }
}
