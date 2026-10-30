//! Third column sizing mode: every visible column fits inside the panel width.
//!
//! Shares the width model of `autofit`: the 8px right margin
//! before `available` matches `columns::project`/`horizontal::required_width`, and the
//! per-column minimums match its clamps (100px for the name column, 64px for the rest).
//!
//! Distribution policy: content columns sit at their measured width and all spare
//! space goes to the name column, exactly like the automatic mode. When the measured
//! columns overflow the panel, the name column absorbs the deficit down to its own
//! minimum first; the other columns then shrink by a shared factor but never below
//! their own content (nothing truncates while any slack remains in the name column),
//! and only in the extreme case do they compress to the fixed minimums, where the
//! existing horizontal scrollbar takes over.

use super::autofit::ColumnMeasurement;
use crate::domain::list_columns::ColumnId;

pub(super) fn min_width(id: ColumnId) -> f32 {
    if id == ColumnId::Name {
        100.0
    } else {
        64.0
    }
}

/// Smallest width that still shows the measured cell text in full: the render
/// budget is `cell.width - 8.0` (`cell_content::render`), so the floor keeps the
/// exact text width inside the budget. Only used for columns other than the name.
pub(super) fn content_floor(measurement: &ColumnMeasurement) -> f32 {
    min_width(measurement.id).max(measurement.content + 8.0)
}

/// Floor used when a column gives up space to a dragged anchor: the name column
/// stays the absorber down to its fixed minimum, while the other columns keep
/// their content complete.
pub(super) fn shrink_floor(measurement: &ColumnMeasurement) -> f32 {
    if measurement.id == ColumnId::Name {
        min_width(measurement.id)
    } else {
        content_floor(measurement)
    }
}

fn clamped_total(
    measured: &[ColumnMeasurement],
    factor: f32,
    floor: impl Fn(&ColumnMeasurement) -> f32,
) -> f32 {
    measured
        .iter()
        .map(|column| floor(column).max(column.width * factor))
        .sum()
}

/// Bisects the shared shrink factor so the columns end exactly on `budget`.
/// Requires `sum(floor) <= budget < sum(width)`.
fn solve(
    measured: &[ColumnMeasurement],
    budget: f32,
    floor: impl Fn(&ColumnMeasurement) -> f32,
) -> Vec<(ColumnId, f32)> {
    let mut low = 0.0_f32;
    let mut high = 1.0_f32;
    for _ in 0..32 {
        let middle = (low + high) * 0.5;
        if clamped_total(measured, middle, &floor) > budget {
            high = middle;
        } else {
            low = middle;
        }
    }
    let mut widths: Vec<(ColumnId, f32)> = measured
        .iter()
        .map(|column| (column.id, floor(column).max(column.width * low)))
        .collect();
    // Float noise can leave the bisection a hair above the budget: trim the
    // excess off the widest column above its floor. When the excess reaches the
    // slack the column lands exactly on its floor, so the fitted total never
    // exceeds the budget (a hair below it is invisible and scrollbar-free).
    let mut total: f32 = widths.iter().map(|(_, width)| *width).sum();
    while total - budget > 0.0 {
        let mut best: Option<(usize, f32)> = None;
        for (index, ((_, width), measurement)) in widths.iter().zip(measured).enumerate() {
            let slack = *width - floor(measurement);
            if slack > 0.0 && best.is_none_or(|(_, best_slack)| slack > best_slack) {
                best = Some((index, slack));
            }
        }
        let Some((index, slack)) = best else {
            break;
        };
        let floor_width = floor(&measured[index]);
        let excess = (total - budget).min(slack);
        widths[index].1 = if widths[index].1 - excess <= floor_width {
            floor_width
        } else {
            widths[index].1 - excess
        };
        let new_total: f32 = widths.iter().map(|(_, width)| *width).sum();
        if new_total >= total {
            break;
        }
        total = new_total;
    }
    widths
}

fn name_minimum(measured: &[ColumnMeasurement]) -> f32 {
    measured
        .iter()
        .find(|column| column.id == ColumnId::Name)
        .map(|column| min_width(column.id))
        .unwrap_or(0.0)
}

/// Merges the solved widths of the non-name columns back over the full measured
/// list, pinning the name column at its minimum.
fn pinned(measured: &[ColumnMeasurement], others: &[(ColumnId, f32)]) -> Vec<(ColumnId, f32)> {
    measured
        .iter()
        .map(|column| {
            let width = if column.id == ColumnId::Name {
                min_width(column.id)
            } else {
                others
                    .iter()
                    .find(|(id, _)| *id == column.id)
                    .map(|(_, width)| *width)
                    .unwrap_or(column.width)
            };
            (column.id, width)
        })
        .collect()
}

/// Sizes the measured columns so that they plus the 8px right margin total `available`.
///
/// Returns the planned widths and whether even the fixed minimums overflowed (the
/// existing horizontal scrollbar must handle the rest in that case).
pub(super) fn allocate_fit_to_panel(
    available: f32,
    measured: &[ColumnMeasurement],
) -> Option<(Vec<(ColumnId, f32)>, bool)> {
    if measured.is_empty() || !available.is_finite() || available <= 8.0 {
        return None;
    }
    let budget = available - 8.0;
    let natural_total = measured.iter().map(|column| column.width).sum::<f32>();
    if natural_total <= budget {
        // Spare space goes to the name column, exactly like the automatic mode.
        let spare = budget - natural_total;
        return Some((
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
            false,
        ));
    }
    let deficit = natural_total - budget;
    let name_slack = measured
        .iter()
        .find(|column| column.id == ColumnId::Name)
        .map_or(0.0, |column| (column.width - min_width(column.id)).max(0.0));
    if deficit <= name_slack {
        // Phase 1: the name column absorbs the whole deficit alone.
        return Some((
            measured
                .iter()
                .map(|column| {
                    (
                        column.id,
                        if column.id == ColumnId::Name {
                            column.width - deficit
                        } else {
                            column.width
                        },
                    )
                })
                .collect(),
            false,
        ));
    }
    // Phase 2: the name column sits at its minimum and the other columns shrink
    // by a shared factor, never below their own content (nothing truncates yet).
    let others: Vec<ColumnMeasurement> = measured
        .iter()
        .filter(|column| column.id != ColumnId::Name)
        .cloned()
        .collect();
    let name_min = name_minimum(measured);
    let others_budget = budget - name_min;
    if others_budget > 0.0 {
        let others_natural = others.iter().map(|column| column.width).sum::<f32>();
        let floors_total = others.iter().map(content_floor).sum::<f32>();
        if others_natural > others_budget && floors_total <= others_budget {
            return Some((
                pinned(measured, &solve(&others, others_budget, content_floor)),
                false,
            ));
        }
        if others_natural <= others_budget {
            // The other columns already fit beside the minimal name column.
            let solved = others
                .iter()
                .map(|column| (column.id, column.width))
                .collect::<Vec<_>>();
            return Some((pinned(measured, &solved), false));
        }
    }
    // Phase 3: below content, down to the fixed minimums of every column.
    let fixed_total = measured
        .iter()
        .map(|column| min_width(column.id))
        .sum::<f32>();
    if fixed_total > budget {
        return Some((
            measured
                .iter()
                .map(|column| (column.id, min_width(column.id)))
                .collect(),
            true,
        ));
    }
    Some((
        solve(measured, budget, |column| min_width(column.id)),
        false,
    ))
}

#[cfg(test)]
#[path = "fit_panel_tests.rs"]
mod tests;
