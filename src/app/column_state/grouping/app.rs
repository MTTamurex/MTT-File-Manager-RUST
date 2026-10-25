//! Focused-view integration and stable signatures shared with Miller ancestor requests.
use super::*;
use std::hash::{Hash, Hasher};

impl ImageViewerApp {
    pub(crate) fn column_group_signature(&mut self, column: ColumnId) -> u64 {
        let path = self.navigation_state.current_path.clone();
        let fingerprint = self.columns.sorting.fingerprint(&self.all_items);
        self.column_group_signature_for(column, &path, self.generation, fingerprint)
    }

    pub(super) fn column_group_signature_for(
        &self,
        column: ColumnId,
        path: &str,
        generation: usize,
        fingerprint: u64,
    ) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut hash);
        generation.hash(&mut hash);
        fingerprint.hash(&mut hash);
        column.hash(&mut hash);
        rust_i18n::locale().to_string().hash(&mut hash);
        (crate::domain::date_format::date_format() as u8).hash(&mut hash);
        crate::application::grouping::local_grouping_day().hash(&mut hash);
        if column == ColumnId::Tags {
            self.tag_assignments_epoch.hash(&mut hash);
            for id in self.sorted_tag_ids.iter() {
                id.hash(&mut hash);
                self.tag_definitions
                    .get(id)
                    .map(|tag| &tag.name)
                    .hash(&mut hash);
            }
        }
        hash.finish()
    }

    pub(crate) fn refresh_column_grouping(&mut self, ctx: &egui::Context) {
        let pane = usize::from(
            matches!(
                self.dual_panel_active,
                crate::app::dual_panel::ActivePanel::Right
            ) ^ self.in_inactive_panel_context,
        );
        self.columns.grouping.poll();
        if !self.dual_panel_enabled {
            self.columns.grouping.set_current(1 - pane, None);
            self.columns
                .grouping
                .retain_ancestors(1 - pane, &HashSet::new());
        }
        if self.view_mode != crate::domain::file_entry::ViewMode::Miller
            || !matches!(self.group_mode, GroupMode::Column(_))
        {
            self.columns
                .grouping
                .retain_ancestors(pane, &HashSet::new());
        }
        let GroupMode::Column(column) = self.group_mode else {
            self.columns.grouping.set_current(pane, None);
            return;
        };
        if self.navigation_state.is_computer_view
            || self.is_loading_folder
            || self.folder_load_error.is_some()
        {
            self.columns.grouping.set_current(pane, None);
            return;
        }
        let signature = self.column_group_signature(column);
        let state = &mut self.columns.grouping;
        let changed = state.set_current(pane, Some(signature));
        if state.needs_request(signature, Instant::now(), ctx) {
            if let Some(sender) = &state.sender {
                let cancelled = Arc::new(AtomicBool::new(false));
                let request = GroupRequest {
                    signature,
                    items: self.all_items.clone(),
                    column,
                    tags: self.tag_assignments_normalized.clone(),
                    definitions: self.tag_definitions.clone(),
                    cancelled: cancelled.clone(),
                    ctx: ctx.clone(),
                };
                if sender.try_send(request).is_ok() {
                    state.record_request(signature, cancelled);
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }
        let busy = self.rectangle_selection_state.is_some()
            || self.is_item_dragging
            || self.external_drop_active
            || self.renaming_state.is_some()
            || self.context_menu.is_open;
        let publish = state.publish_ready(signature, pane, busy, &self.group_projection);
        if !busy && (changed || publish) {
            self.rebuild_group_projection();
        }
    }
}
