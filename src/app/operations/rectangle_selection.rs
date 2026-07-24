use eframe::egui;
use std::path::PathBuf;

use crate::app::state::ImageViewerApp;
use crate::ui::cache::FxHashSet;
use crate::ui::views::rectangle_selection::{
    collect_indices_in_rect, RectangleSelectionFrame, RectangleSelectionMetrics,
    RectangleSelectionModifiers, RectangleSelectionSource, RectangleSelectionState,
};

pub(crate) struct RectangleSelectionResolveResult {
    pub selected_paths: FxHashSet<PathBuf>,
    pub focus_index: Option<usize>,
    pub anchor_index: Option<usize>,
    pub selection_changed: bool,
}

pub(crate) fn resolve_rectangle_selection(
    item_count: usize,
    base_selection: &FxHashSet<PathBuf>,
    hit_indices: &FxHashSet<usize>,
    anchor: Option<usize>,
    modifiers: RectangleSelectionModifiers,
    get_item_path: impl Fn(usize) -> Option<PathBuf>,
) -> RectangleSelectionResolveResult {
    let mut hits: Vec<usize> = hit_indices
        .iter()
        .copied()
        .filter(|idx| *idx < item_count)
        .collect();
    hits.sort_unstable();

    if hits.is_empty() {
        return RectangleSelectionResolveResult {
            selected_paths: base_selection.clone(),
            focus_index: None,
            anchor_index: anchor,
            selection_changed: false,
        };
    }

    let focus_index = hits.last().copied();
    let anchor_index = if modifiers.shift {
        anchor.or(focus_index)
    } else {
        focus_index
    };

    let mut selected_paths = if modifiers.ctrl || modifiers.shift {
        base_selection.clone()
    } else {
        FxHashSet::default()
    };

    if modifiers.ctrl {
        for idx in hits {
            if let Some(path) = get_item_path(idx) {
                if selected_paths.contains(&path) {
                    selected_paths.remove(&path);
                } else {
                    selected_paths.insert(path);
                }
            }
        }
    } else if modifiers.shift {
        if let (Some(anchor_idx), Some(first), Some(last)) =
            (anchor, hits.first().copied(), hits.last().copied())
        {
            let (start, end) = if anchor_idx < first {
                (anchor_idx, last)
            } else if anchor_idx > last {
                (first, anchor_idx)
            } else {
                (first, last)
            };

            for idx in start..=end.min(item_count.saturating_sub(1)) {
                if let Some(path) = get_item_path(idx) {
                    selected_paths.insert(path);
                }
            }
        } else {
            for idx in hits {
                if let Some(path) = get_item_path(idx) {
                    selected_paths.insert(path);
                }
            }
        }
    } else {
        for idx in hits {
            if let Some(path) = get_item_path(idx) {
                selected_paths.insert(path);
            }
        }
    }

    let selection_changed = selected_paths != *base_selection;

    RectangleSelectionResolveResult {
        selected_paths,
        focus_index: selection_changed.then_some(focus_index).flatten(),
        anchor_index: if selection_changed {
            anchor_index
        } else {
            anchor
        },
        selection_changed,
    }
}

pub(crate) fn resolve_rectangle_preview_indices(
    item_count: usize,
    base_preview_indices: &FxHashSet<usize>,
    hit_indices: &FxHashSet<usize>,
    anchor: Option<usize>,
    modifiers: RectangleSelectionModifiers,
) -> FxHashSet<usize> {
    let mut hits: Vec<usize> = hit_indices
        .iter()
        .copied()
        .filter(|idx| *idx < item_count)
        .collect();
    hits.sort_unstable();

    if hits.is_empty() {
        return base_preview_indices.clone();
    }

    if !modifiers.ctrl && !modifiers.shift {
        return hits.into_iter().collect();
    }

    let mut preview_indices = base_preview_indices.clone();

    if modifiers.ctrl {
        for idx in hits {
            if !preview_indices.remove(&idx) {
                preview_indices.insert(idx);
            }
        }
    } else if let (Some(anchor_idx), Some(first), Some(last)) =
        (anchor, hits.first().copied(), hits.last().copied())
    {
        let (start, end) = if anchor_idx < first {
            (anchor_idx, last)
        } else if anchor_idx > last {
            (first, anchor_idx)
        } else {
            (first, last)
        };

        for idx in start..=end.min(item_count.saturating_sub(1)) {
            preview_indices.insert(idx);
        }
    } else {
        preview_indices.extend(hits);
    }

    preview_indices
}

impl ImageViewerApp {
    pub fn clear_file_view_selection(&mut self) {
        self.miller_columns.clear_selection_anchors();
        if self.selected_item.is_none()
            && self.selected_file.is_none()
            && self.multi_selection.is_empty()
            && self.selection_anchor.is_none()
            && self.selected_metadata.is_none()
            && !self.scroll_to_selected
        {
            return;
        }

        self.multi_selection.clear();
        self.selected_item = None;
        self.selected_file = None;
        self.selection_anchor = None;
        self.selected_metadata = None;
        self.scroll_to_selected = false;
        self.update_selected_thumbnail();
        self.ui_ctx.request_repaint();
    }

    pub fn handle_rectangle_selection_frame(
        &mut self,
        ui: &egui::Ui,
        frame: &RectangleSelectionFrame,
        suppress_new_start: bool,
    ) {
        if self
            .rectangle_selection_state
            .as_ref()
            .is_some_and(|state| state.source != RectangleSelectionSource::CurrentItems)
        {
            if !matches!(self.view_mode, crate::domain::file_entry::ViewMode::Miller) {
                self.cancel_rectangle_selection();
            }
            return;
        }

        let Some(metrics) = frame.metrics else {
            if self.rectangle_selection_state.is_some() {
                self.cancel_rectangle_selection();
            }
            return;
        };
        let view = metrics.view();

        if self
            .rectangle_selection_state
            .as_ref()
            .is_some_and(|state| state.view != view || state.generation != self.generation)
        {
            self.cancel_rectangle_selection();
        }

        if self.renaming_state.is_some() || self.is_item_dragging {
            if self.rectangle_selection_state.is_some() {
                self.cancel_rectangle_selection();
            }
            return;
        }

        if self.rectangle_selection_state.is_none() && !suppress_new_start {
            if let Some(start_screen_pos) = frame.start_screen_pos {
                if let Some(anchor_content) = frame.screen_to_content(start_screen_pos) {
                    let modifiers = ui.input(|input| RectangleSelectionModifiers {
                        ctrl: input.modifiers.ctrl,
                        shift: input.modifiers.shift,
                    });
                    let base_selection = self.multi_selection.clone();
                    let base_preview_indices = self.indices_for_selected_paths(&base_selection);
                    self.rectangle_selection_state = Some(RectangleSelectionState::new(
                        view,
                        anchor_content,
                        base_selection,
                        base_preview_indices,
                        modifiers,
                        self.generation,
                    ));
                }
            }
        }

        if !self
            .rectangle_selection_state
            .as_ref()
            .is_some_and(|state| state.view == view)
        {
            return;
        }

        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.cancel_rectangle_selection();
            return;
        }

        let pointer_pos = ui.ctx().pointer_latest_pos().or(frame.start_screen_pos);
        if let Some(pointer_pos) = pointer_pos.and_then(|pos| frame.screen_to_content(pos)) {
            if let Some(state) = self.rectangle_selection_state.as_mut() {
                state.current_content = pointer_pos;
            }
        }

        self.apply_rectangle_selection_autoscroll(ui, frame);
        self.update_rectangle_selection_preview(metrics);

        let primary_released = ui.input(|input| input.pointer.primary_released());
        if primary_released {
            self.finish_rectangle_selection();
        } else {
            ui.ctx().request_repaint();
        }
    }

    pub fn cancel_rectangle_selection(&mut self) {
        if self.rectangle_selection_state.take().is_some() {
            self.ui_ctx.request_repaint();
        }
    }

    fn apply_rectangle_selection_autoscroll(
        &mut self,
        ui: &egui::Ui,
        frame: &RectangleSelectionFrame,
    ) {
        let Some(viewport) = frame.viewport_rect else {
            return;
        };
        let Some(pointer_pos) = ui.ctx().pointer_latest_pos() else {
            return;
        };

        const EDGE_ZONE: f32 = 48.0;
        const MAX_STEP_PER_FRAME: f32 = 34.0;

        let top_distance = pointer_pos.y - viewport.top();
        let bottom_distance = viewport.bottom() - pointer_pos.y;
        let up = ((EDGE_ZONE - top_distance) / EDGE_ZONE).clamp(0.0, 1.0);
        let down = ((EDGE_ZONE - bottom_distance) / EDGE_ZONE).clamp(0.0, 1.0);
        let delta = (down - up) * MAX_STEP_PER_FRAME;

        if frame.max_scroll_y > 0.0 && delta.abs() > 0.1 {
            self.scroll_offset_y = (self.scroll_offset_y + delta).clamp(0.0, frame.max_scroll_y);
        }

        let left_distance = pointer_pos.x - viewport.left();
        let right_distance = viewport.right() - pointer_pos.x;
        let left = ((EDGE_ZONE - left_distance) / EDGE_ZONE).clamp(0.0, 1.0);
        let right = ((EDGE_ZONE - right_distance) / EDGE_ZONE).clamp(0.0, 1.0);
        let delta_x = (right - left) * MAX_STEP_PER_FRAME;
        if frame.max_scroll_x > 0.0 && delta_x.abs() > 0.1 {
            self.scroll_offset_x = (self.scroll_offset_x + delta_x).clamp(0.0, frame.max_scroll_x);
        }

        if (frame.max_scroll_y > 0.0 && delta.abs() > 0.1)
            || (frame.max_scroll_x > 0.0 && delta_x.abs() > 0.1)
        {
            ui.ctx().request_repaint();
        }
    }

    fn update_rectangle_selection_preview(&mut self, metrics: RectangleSelectionMetrics) {
        let Some(state) = self.rectangle_selection_state.as_ref() else {
            return;
        };
        let selection_rect = state.content_rect();
        let modifiers = state.modifiers;
        let hit_indices = collect_indices_in_rect(selection_rect, metrics);
        let preview_indices = resolve_rectangle_preview_indices(
            self.items.len(),
            &state.base_preview_indices,
            &hit_indices,
            self.selection_anchor,
            modifiers,
        );

        if let Some(state) = self.rectangle_selection_state.as_mut() {
            state.hit_indices = hit_indices;
            state.preview_indices = preview_indices;
        }
    }

    fn finish_rectangle_selection(&mut self) {
        let Some(state) = self.rectangle_selection_state.take() else {
            return;
        };

        let resolved = resolve_rectangle_selection(
            self.items.len(),
            &state.base_selection,
            &state.hit_indices,
            self.selection_anchor,
            state.modifiers,
            |idx| self.items.get(idx).map(|item| item.path.clone()),
        );

        if !resolved.selection_changed {
            self.ui_ctx.request_repaint();
            return;
        }

        self.multi_selection = resolved.selected_paths;
        self.selection_anchor = resolved.anchor_index;

        if let Some(focus_idx) = resolved.focus_index {
            if let Some(item) = self.items.get(focus_idx) {
                self.selected_item = Some(focus_idx);
                self.selected_file = Some(item.clone());
                self.update_selected_thumbnail();
            }
        }

        self.ui_ctx.request_repaint();
    }

    fn indices_for_selected_paths(&self, selected_paths: &FxHashSet<PathBuf>) -> FxHashSet<usize> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| selected_paths.contains(&item.path).then_some(idx))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(index: usize) -> PathBuf {
        PathBuf::from(format!("C:/tmp/item-{index}"))
    }

    fn paths(count: usize) -> Vec<PathBuf> {
        (0..count).map(path).collect()
    }

    fn set(indices: &[usize]) -> FxHashSet<usize> {
        indices.iter().copied().collect()
    }

    fn path_set(indices: &[usize]) -> FxHashSet<PathBuf> {
        indices.iter().copied().map(path).collect()
    }

    #[test]
    fn simple_rectangle_replaces_selection() {
        let items = paths(6);
        let result = resolve_rectangle_selection(
            items.len(),
            &path_set(&[0, 1]),
            &set(&[2, 4]),
            Some(1),
            RectangleSelectionModifiers::default(),
            |idx| items.get(idx).cloned(),
        );

        assert_eq!(result.selected_paths, path_set(&[2, 4]));
        assert_eq!(result.focus_index, Some(4));
        assert_eq!(result.anchor_index, Some(4));
    }

    #[test]
    fn ctrl_rectangle_toggles_against_base_selection() {
        let items = paths(6);
        let result = resolve_rectangle_selection(
            items.len(),
            &path_set(&[1, 2, 5]),
            &set(&[2, 3, 4]),
            Some(1),
            RectangleSelectionModifiers {
                ctrl: true,
                shift: false,
            },
            |idx| items.get(idx).cloned(),
        );

        assert_eq!(result.selected_paths, path_set(&[1, 3, 4, 5]));
        assert_eq!(result.focus_index, Some(4));
        assert_eq!(result.anchor_index, Some(4));
    }

    #[test]
    fn shift_rectangle_extends_range_from_anchor() {
        let items = paths(8);
        let result = resolve_rectangle_selection(
            items.len(),
            &path_set(&[0]),
            &set(&[4, 5]),
            Some(2),
            RectangleSelectionModifiers {
                ctrl: false,
                shift: true,
            },
            |idx| items.get(idx).cloned(),
        );

        assert_eq!(result.selected_paths, path_set(&[0, 2, 3, 4, 5]));
        assert_eq!(result.focus_index, Some(5));
        assert_eq!(result.anchor_index, Some(2));
    }

    #[test]
    fn empty_rectangle_keeps_base_selection() {
        let items = paths(4);
        let result = resolve_rectangle_selection(
            items.len(),
            &path_set(&[1]),
            &FxHashSet::default(),
            Some(1),
            RectangleSelectionModifiers::default(),
            |idx| items.get(idx).cloned(),
        );

        assert_eq!(result.selected_paths, path_set(&[1]));
        assert_eq!(result.focus_index, None);
        assert_eq!(result.anchor_index, Some(1));
        assert!(!result.selection_changed);
    }

    #[test]
    fn identical_resolved_selection_is_reported_unchanged() {
        let items = paths(6);
        let result = resolve_rectangle_selection(
            items.len(),
            &path_set(&[2, 4]),
            &set(&[2, 4]),
            Some(1),
            RectangleSelectionModifiers::default(),
            |idx| items.get(idx).cloned(),
        );

        assert_eq!(result.selected_paths, path_set(&[2, 4]));
        assert_eq!(result.focus_index, None);
        assert_eq!(result.anchor_index, Some(1));
        assert!(!result.selection_changed);
    }

    #[test]
    fn preview_indices_resolve_without_path_rebuild() {
        let preview = resolve_rectangle_preview_indices(
            8,
            &set(&[0]),
            &set(&[4, 5]),
            Some(2),
            RectangleSelectionModifiers {
                ctrl: false,
                shift: true,
            },
        );

        assert_eq!(preview, set(&[0, 2, 3, 4, 5]));
    }
}
