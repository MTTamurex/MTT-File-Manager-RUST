use eframe::egui;

use crate::app::operations::rectangle_selection::{
    resolve_rectangle_preview_indices, resolve_rectangle_selection,
};
use crate::app::state::ImageViewerApp;
use crate::domain::file_entry::FileEntry;
use crate::ui::cache::FxHashSet;
use crate::ui::views::rectangle_selection::{
    collect_indices_in_rect, RectangleSelectionFrame, RectangleSelectionModifiers,
    RectangleSelectionSource, RectangleSelectionState,
};

impl ImageViewerApp {
    pub(super) fn handle_miller_rectangle_selection_frame(
        &mut self,
        ui: &egui::Ui,
        frame: &RectangleSelectionFrame,
        items: &[FileEntry],
    ) {
        let Some(metrics) = frame.metrics else {
            return;
        };
        let RectangleSelectionSource::MillerAncestor {
            directory,
            listing_id,
        } = &frame.source
        else {
            return;
        };

        if self
            .rectangle_selection_state
            .as_ref()
            .is_some_and(|state| {
                matches!(
                    &state.source,
                    RectangleSelectionSource::MillerAncestor {
                        directory: active_directory,
                        listing_id: active_listing_id,
                    } if active_directory == directory && active_listing_id != listing_id
                )
            })
        {
            self.cancel_rectangle_selection();
        }

        if self.rectangle_selection_state.is_none() {
            if let Some(start_screen_pos) = frame.start_screen_pos {
                let Some(anchor_content) = frame.screen_to_content(start_screen_pos) else {
                    return;
                };
                let modifiers = ui.input(|input| RectangleSelectionModifiers {
                    ctrl: input.modifiers.ctrl,
                    shift: input.modifiers.shift,
                });
                let base_selection: FxHashSet<_> = items
                    .iter()
                    .filter(|item| self.multi_selection.contains(&item.path))
                    .map(|item| item.path.clone())
                    .collect();
                let base_preview_indices = items
                    .iter()
                    .enumerate()
                    .filter_map(|(index, item)| {
                        base_selection.contains(&item.path).then_some(index)
                    })
                    .collect();

                self.multi_selection.clone_from(&base_selection);
                self.selected_item = None;
                self.selection_anchor = None;
                if self
                    .selected_file
                    .as_ref()
                    .is_some_and(|selected| !base_selection.contains(&selected.path))
                {
                    self.selected_file = None;
                }
                self.update_selected_thumbnail();
                self.rectangle_selection_state = Some(RectangleSelectionState::new_for_source(
                    metrics.view(),
                    frame.source.clone(),
                    anchor_content,
                    base_selection,
                    base_preview_indices,
                    modifiers,
                    *listing_id,
                ));
            }
        }

        if !self
            .rectangle_selection_state
            .as_ref()
            .is_some_and(|state| state.source == frame.source)
        {
            return;
        }

        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.cancel_rectangle_selection();
            return;
        }

        if let Some(pointer_content) = ui
            .ctx()
            .pointer_latest_pos()
            .or(frame.start_screen_pos)
            .and_then(|position| frame.screen_to_content(position))
        {
            if let Some(state) = self.rectangle_selection_state.as_mut() {
                state.current_content = pointer_content;
            }
        }

        let Some(state) = self.rectangle_selection_state.as_ref() else {
            return;
        };
        let hit_indices = collect_indices_in_rect(state.content_rect(), metrics);
        let preview_indices = resolve_rectangle_preview_indices(
            items.len(),
            &state.base_preview_indices,
            &hit_indices,
            None,
            state.modifiers,
        );
        if let Some(state) = self.rectangle_selection_state.as_mut() {
            state.hit_indices = hit_indices;
            state.preview_indices = preview_indices;
        }

        if ui.input(|input| input.pointer.primary_released()) {
            self.finish_miller_rectangle_selection(items);
        } else {
            ui.ctx().request_repaint();
        }
    }

    fn finish_miller_rectangle_selection(&mut self, items: &[FileEntry]) {
        let Some(state) = self.rectangle_selection_state.take() else {
            return;
        };
        let resolved = resolve_rectangle_selection(
            items.len(),
            &state.base_selection,
            &state.hit_indices,
            None,
            state.modifiers,
            |index| items.get(index).map(|item| item.path.clone()),
        );

        if !resolved.selection_changed {
            self.ui_ctx.request_repaint();
            return;
        }

        self.multi_selection = resolved.selected_paths;
        self.selected_item = None;
        self.selection_anchor = None;
        self.selected_file = resolved
            .focus_index
            .and_then(|index| items.get(index))
            .filter(|item| self.multi_selection.contains(&item.path))
            .cloned()
            .or_else(|| {
                items
                    .iter()
                    .rev()
                    .find(|item| self.multi_selection.contains(&item.path))
                    .cloned()
            });
        self.update_selected_thumbnail();
        self.ui_ctx.request_repaint();
    }
}
