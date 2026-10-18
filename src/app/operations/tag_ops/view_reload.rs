use crate::app::state::ImageViewerApp;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, PartialEq, Eq)]
struct TagViewLoadPolicy {
    reset_view: bool,
    preserve_listing: bool,
}

fn tag_view_load_policy(
    current_path: &str,
    view_path: &str,
    visible_count: usize,
) -> TagViewLoadPolicy {
    let same_view = current_path == view_path;
    TagViewLoadPolicy {
        reset_view: !same_view,
        preserve_listing: same_view && visible_count > 0,
    }
}

pub(super) fn defer_tag_refilter(is_loading: bool, hold_visible_items: bool) -> bool {
    is_loading && hold_visible_items
}

pub(super) fn tag_change_requires_reload(is_loading: bool, missing_items: bool) -> bool {
    // The worker may already have queried assignments that were just removed.
    // Restart its generation instead of publishing a mixed old/new tag listing.
    is_loading || missing_items
}

impl ImageViewerApp {
    pub(super) fn prepare_tag_view_loading(&mut self, tag_id: i64, view_path: String) {
        let policy = tag_view_load_policy(
            &self.navigation_state.current_path,
            &view_path,
            self.items.len(),
        );
        self.bump_folder_load_generation();
        self.invalidate_active_items_rebuild();
        self.navigation_state.current_path = view_path.clone();
        self.navigation_state.path_input = self.tag_view_display_name(tag_id);
        self.navigation_state.is_computer_view = false;
        self.navigation_state.is_recycle_bin_view = false;
        self.active_tag_filter = Some(tag_id);

        if policy.reset_view {
            // New views need cover discovery; same-view reloads retain the shared
            // visual pipeline so the other pane does not lose in-flight previews.
            self.scanned_folders.clear();
            if self.uses_conservative_folder_preview_policy() {
                self.cache_manager.clear_folder_preview_inflight_state();
                self.pending_folder_preview_replace.clear();
                self.suppress_next_folder_preview_invalidation.clear();
            }
            self.reset_selection_and_search();
            self.apply_folder_lock_if_present();
        }
        if !policy.preserve_listing {
            self.items = Arc::new(Vec::new());
            self.group_projection = Arc::new(Default::default());
            self.all_items_mut().clear();
            self.total_items = 0;
            self.stale_items_snapshot = None;
        }

        self.is_loading_folder = true;
        self.folder_load_error = None;
        // The streaming receiver clears all_items when the first new batch
        // arrives (or at end-of-load for an empty result), then swaps the final
        // listing into items. No updates are skipped to avoid the visual flash.
        self.pending_all_items_clear = policy.preserve_listing;
        self.hold_visible_items_until_load_complete = policy.preserve_listing;
        self.pending_items_rebuild = false;
        self.pending_items_count = 0;
        self.loading_started_at = Instant::now();
        self.loaded_path = view_path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::special_paths::tag_view_path;

    #[test]
    fn same_tag_reload_preserves_the_visible_listing() {
        let path = tag_view_path(1);
        assert_eq!(
            tag_view_load_policy(&path, &path, 10),
            TagViewLoadPolicy {
                reset_view: false,
                preserve_listing: true,
            }
        );
    }

    #[test]
    fn navigation_and_empty_tag_views_still_load_incrementally() {
        let path = tag_view_path(1);
        for current in [tag_view_path(2), "folder".to_owned()] {
            assert_eq!(
                tag_view_load_policy(&current, &path, 10),
                TagViewLoadPolicy {
                    reset_view: true,
                    preserve_listing: false,
                }
            );
        }
        assert_eq!(
            tag_view_load_policy(&path, &path, 0),
            TagViewLoadPolicy {
                reset_view: false,
                preserve_listing: false,
            }
        );
    }

    #[test]
    fn refilter_waits_for_the_complete_replacement_of_a_preserved_listing() {
        assert!(defer_tag_refilter(true, true));
        assert!(!defer_tag_refilter(true, false));
        assert!(!defer_tag_refilter(false, true));
        assert!(!defer_tag_refilter(false, false));
    }

    #[test]
    fn assignment_changes_restart_in_flight_tag_loads_even_without_missing_items() {
        assert!(tag_change_requires_reload(true, false));
        assert!(tag_change_requires_reload(true, true));
        assert!(tag_change_requires_reload(false, true));
        assert!(!tag_change_requires_reload(false, false));
    }
}
