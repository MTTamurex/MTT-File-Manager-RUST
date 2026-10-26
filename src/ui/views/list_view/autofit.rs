//! Content-sized columns with pane-local widths and cache-only metadata measurement.
use super::{autofit_layout, cell_content, columns, name_cell, text_layout, ListViewContext};
use crate::domain::list_columns::{ColumnId, ColumnScope};
use eframe::egui::{Color32, FontId, Ui};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColumnMeasurement {
    pub id: ColumnId,
    pub width: f32,
}

impl ColumnMeasurement {
    pub(crate) fn new(ui: &Ui, id: ColumnId, label: &str) -> Self {
        let width = text_layout::get_cached_text_width(
            label,
            &FontId::proportional(12.0),
            Color32::WHITE,
            ui,
        );
        Self {
            id,
            width: (width + 30.0).max(if id == ColumnId::Name { 100.0 } else { 80.0 }),
        }
    }

    pub(crate) fn include(&mut self, ui: &Ui, text: &str, padding: f32) {
        if text.is_empty() {
            return;
        }
        let width = text_layout::get_cached_text_width(
            text,
            &FontId::proportional(12.0),
            Color32::WHITE,
            ui,
        );
        self.width = self.width.max(width + padding);
    }
}

/// Only spare space is distributed. Measured content is never compressed to fit a viewport.
pub(crate) fn allocate(
    available: f32,
    measured: &[ColumnMeasurement],
) -> Option<Vec<(ColumnId, f32)>> {
    if measured.is_empty() || !available.is_finite() || available <= 8.0 {
        return None;
    }
    let spare =
        (available - 8.0 - measured.iter().map(|column| column.width).sum::<f32>()).max(0.0);
    Some(
        measured
            .iter()
            .map(|column| {
                (
                    column.id,
                    column.width
                        + if column.id == ColumnId::Name {
                            spare
                        } else {
                            0.0
                        },
                )
            })
            .collect(),
    )
}

#[derive(Clone, Default)]
struct FitState {
    active: bool,
    scope: Option<ColumnScope>,
    input: u64,
    source: u64,
    natural: Vec<ColumnMeasurement>,
    planned: Vec<(ColumnId, f32)>,
    visible: Vec<usize>,
}

impl FitState {
    fn fit(
        &mut self,
        input: u64,
        source: u64,
        available: f32,
        mut measured: Vec<ColumnMeasurement>,
        force: bool,
    ) {
        // Late properties and newly visible rows may grow widths, never make them jitter smaller.
        if self.active && self.source == source && !force {
            for column in &mut measured {
                if let Some(previous) = self
                    .natural
                    .iter()
                    .find(|previous| previous.id == column.id)
                {
                    column.width = column.width.max(previous.width);
                }
            }
        }
        if let Some(planned) = allocate(available, &measured) {
            self.planned = planned;
            self.natural = measured;
            self.input = input;
            self.source = source;
            self.active = true;
        }
    }
}

fn signature(
    ctx: &ListViewContext,
    ids: &[ColumnId],
    available: f32,
    indices: &[usize],
) -> (u64, u64) {
    let mut source = std::collections::hash_map::DefaultHasher::new();
    ctx.generation.hash(&mut source);
    ctx.items_revision.hash(&mut source);
    (ctx.items.as_ptr() as usize).hash(&mut source);
    ctx.items.len().hash(&mut source);
    columns::scope(ctx).hash(&mut source);
    // Column order and manual widths do not invalidate content measurements.
    ids.iter()
        .map(|id| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            id.hash(&mut hash);
            hash.finish()
        })
        .fold(0_u64, u64::wrapping_add)
        .hash(&mut source);
    (crate::domain::date_format::date_format() as u8).hash(&mut source);
    rust_i18n::locale().to_string().hash(&mut source);
    text_layout::font_metrics_revision().hash(&mut source);
    let source = source.finish();
    let mut input = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut input);
    available.to_bits().hash(&mut input);
    ctx.columns.content_revision.hash(&mut input);
    ctx.columns.catalog_loaded.hash(&mut input);
    ctx.tag_assignments_epoch.hash(&mut input);
    ctx.tag_definitions
        .iter()
        .map(|(id, tag)| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            id.hash(&mut hash);
            tag.name.hash(&mut hash);
            hash.finish()
        })
        .fold(0_u64, u64::wrapping_add)
        .hash(&mut input);
    indices.hash(&mut input);
    if ids.contains(&ColumnId::Size) {
        size_signature(
            indices
                .iter()
                .map(|&index| cell_content::cached_size(&ctx.items[index], ctx)),
        )
        .hash(&mut input);
    }
    (source, input.finish())
}

fn size_signature(values: impl Iterator<Item = Option<u64>>) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for value in values {
        value.hash(&mut hash);
    }
    hash.finish()
}

fn sample_indices(item_count: usize, column_count: usize, visible: &[usize]) -> Vec<usize> {
    let baseline = (2000 / column_count.max(1)).min(200).min(item_count);
    let mut indices: Vec<_> = (0..baseline).collect();
    indices.extend(
        visible
            .iter()
            .copied()
            .filter(|&index| index >= baseline && index < item_count),
    );
    indices.sort_unstable();
    indices.dedup();
    indices
}

fn measure(
    ui: &Ui,
    ctx: &mut ListViewContext,
    ids: &[ColumnId],
    indices: &[usize],
) -> Vec<ColumnMeasurement> {
    let items = ctx.items;
    ids.iter()
        .map(|&id| {
            // Do not size a pending property header using its temporary GUID fallback label.
            let label = if matches!(id, ColumnId::Property(_)) && !ctx.columns.catalog_loaded {
                String::new()
            } else {
                columns::label(ctx, id)
            };
            let mut measurement = ColumnMeasurement::new(ui, id, &label);
            for &index in indices {
                let item = &items[index];
                let padding = if id == ColumnId::Name {
                    30.0 + if name_cell::first_tag_color(item, ctx).is_some() {
                        12.0
                    } else {
                        0.0
                    }
                } else {
                    16.0
                };
                let value = cell_content::text(ui, item, ctx, id, false);
                measurement.include(ui, &value, padding);
            }
            measurement
        })
        .collect()
}

pub(super) fn update(ui: &Ui, ctx: &mut ListViewContext) {
    if ctx.compact {
        return;
    }
    let state_id = ui.id().with("list_content_fit");
    let mut state = ui
        .data_mut(|data| data.get_temp::<FitState>(state_id))
        .unwrap_or_default();
    let scope = columns::scope(ctx);
    if !ctx.auto_resize_columns {
        if ctx.folder_width_location.is_none() && state.active && state.scope == Some(scope) {
            // Switching off automatic sizing keeps the last displayed sizes as manual widths.
            for &(id, width) in &state.planned {
                columns::set_width(ctx, id, width);
            }
        }
        state.active = false;
        ui.data_mut(|data| data.insert_temp(state_id, state));
        return;
    }
    let available = ui.available_width();
    let ids: Vec<_> = columns::cells(ctx).iter().map(|cell| cell.id).collect();
    let indices = sample_indices(ctx.items.len(), ids.len(), &state.visible);
    let (source, input) = signature(ctx, &ids, available, &indices);
    let requested =
        *ctx.autofit_requested || !state.active || autofit_layout::needs_refit(ui, false);
    let busy = ui.ctx().dragged_id().is_some()
        || ctx.is_item_dragging
        || ctx.rectangle_selection_state.is_some()
        || ctx.renaming_state.is_some();
    if available.is_finite() && available > 8.0 && !busy && (requested || input != state.input) {
        state.fit(
            input,
            source,
            available,
            measure(ui, ctx, &ids, &indices),
            requested,
        );
        state.scope = Some(scope);
        *ctx.autofit_requested = false;
        autofit_layout::record_fitted_width(ui);
    }
    if state.active && state.scope == Some(scope) {
        columns::set_fitted_widths(ctx, &state.planned);
    }
    ui.data_mut(|data| data.insert_temp(state_id, state));
}

/// Keep the actual visible item indices, including discontinuous grouped rows.
pub(super) fn remember_visible_rows(ui: &Ui, ctx: &mut ListViewContext) {
    if ctx.compact || !ctx.auto_resize_columns {
        return;
    }
    record_visible_rows(ui, &mut ctx.autofit_visible_indices);
}

fn record_visible_rows(ui: &Ui, visible: &mut Vec<usize>) {
    let id = ui.id().with("list_content_fit");
    visible.sort_unstable();
    visible.dedup();
    let changed = ui.data_mut(|data| {
        if let Some(mut state) = data.get_temp::<FitState>(id) {
            if state.visible != *visible {
                state.visible = std::mem::take(visible);
                data.insert_temp(id, state);
                return true;
            }
        }
        false
    });
    // Repainting locks the egui context too: never call it from inside data_mut.
    if changed {
        ui.ctx().request_repaint();
    }
}

#[cfg(test)]
#[path = "autofit_tests.rs"]
mod tests;
