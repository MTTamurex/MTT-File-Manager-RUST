//! Sorting state and comparisons for the disk analyzer's largest-items table.

use super::disk_analysis_model::DiskAnalysisModel;
use super::disk_analysis_query::{ActiveWeights, SizeMetric};
use super::disk_analysis_state::DiskAnalysisState;
use std::cmp::Ordering as SortOrdering;
use std::sync::Arc;

pub const MAX_LARGEST_SORT_CRITERIA: usize = 2;

/// Sortable columns of the Largest table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LargestColumn {
    Name,
    Path,
    Type,
    Logical,
    Allocated,
    Difference,
    Files,
}

impl LargestColumn {
    pub const fn default_ascending(self) -> bool {
        matches!(self, Self::Name | Self::Path | Self::Type)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargestSortCriterion {
    pub column: LargestColumn,
    pub ascending: bool,
}

impl DiskAnalysisState {
    /// Replace the largest-items sort with a single metric-driven criterion.
    pub fn reset_largest_sort(&mut self, column: LargestColumn) {
        self.largest_sort_criteria = vec![LargestSortCriterion {
            column,
            ascending: column.default_ascending(),
        }];
    }

    /// Update the largest-items sort from a header click. A shift-click adds
    /// a secondary criterion, while a normal click replaces the criteria.
    pub fn update_largest_sort(&mut self, column: LargestColumn, additive: bool) {
        self.largest_sort_criteria
            .truncate(MAX_LARGEST_SORT_CRITERIA);
        if additive {
            if let Some(criterion) = self
                .largest_sort_criteria
                .iter_mut()
                .find(|criterion| criterion.column == column)
            {
                criterion.ascending = !criterion.ascending;
            } else if self.largest_sort_criteria.len() < MAX_LARGEST_SORT_CRITERIA {
                self.largest_sort_criteria.push(LargestSortCriterion {
                    column,
                    ascending: column.default_ascending(),
                });
            }
        } else {
            let ascending = self
                .largest_sort_criteria
                .first()
                .filter(|criterion| criterion.column == column)
                .map_or(column.default_ascending(), |criterion| !criterion.ascending);
            self.largest_sort_criteria = vec![LargestSortCriterion { column, ascending }];
        }
        self.sort_largest_rows();
    }

    /// Sort the largest-rows projection by the current criteria.
    /// Reads sizes straight from the model; rows are just indices.
    pub fn sort_largest_rows(&mut self) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let criteria: Vec<LargestSortCriterion> = self
            .largest_sort_criteria
            .iter()
            .copied()
            .take(MAX_LARGEST_SORT_CRITERIA)
            .collect();
        let metric = self.metric;
        let filtered = self.active_weights.clone();
        let mut rows: Vec<u32> = self.largest_rows.as_ref().clone();
        rows.sort_by(|&a, &b| {
            for criterion in &criteria {
                let mut ord = compare_largest_column(
                    &model,
                    a,
                    b,
                    criterion.column,
                    metric,
                    filtered.as_ref(),
                );
                if !criterion.ascending {
                    ord = ord.reverse();
                }
                if ord != SortOrdering::Equal {
                    return ord;
                }
            }
            a.cmp(&b)
        });
        self.largest_rows = Arc::new(rows);
    }
}

fn compare_largest_column(
    model: &DiskAnalysisModel,
    a: u32,
    b: u32,
    column: LargestColumn,
    metric: SizeMetric,
    filtered: Option<&ActiveWeights>,
) -> SortOrdering {
    let filtered_value =
        |idx: u32| filtered.and_then(|weights| weights.weights.get(idx as usize).copied());
    match column {
        LargestColumn::Name => model.nodes[a as usize]
            .name
            .to_lowercase()
            .cmp(&model.nodes[b as usize].name.to_lowercase()),
        LargestColumn::Path => model.path_of(a).cmp(&model.path_of(b)),
        LargestColumn::Type => model.nodes[b as usize]
            .is_dir
            .cmp(&model.nodes[a as usize].is_dir),
        LargestColumn::Logical if metric == SizeMetric::Logical => filtered_value(a)
            .unwrap_or(model.nodes[a as usize].subtree_size)
            .cmp(&filtered_value(b).unwrap_or(model.nodes[b as usize].subtree_size)),
        LargestColumn::Logical => model.nodes[a as usize]
            .subtree_size
            .cmp(&model.nodes[b as usize].subtree_size),
        LargestColumn::Allocated if metric == SizeMetric::Allocated => filtered_value(a)
            .unwrap_or(model.nodes[a as usize].subtree_allocated_size)
            .cmp(&filtered_value(b).unwrap_or(model.nodes[b as usize].subtree_allocated_size)),
        LargestColumn::Allocated => model.nodes[a as usize]
            .subtree_allocated_size
            .cmp(&model.nodes[b as usize].subtree_allocated_size),
        LargestColumn::Difference => signed_difference(model, a).cmp(&signed_difference(model, b)),
        LargestColumn::Files if metric == SizeMetric::FileCount => filtered_value(a)
            .unwrap_or(model.nodes[a as usize].subtree_files)
            .cmp(&filtered_value(b).unwrap_or(model.nodes[b as usize].subtree_files)),
        LargestColumn::Files => model.nodes[a as usize]
            .subtree_files
            .cmp(&model.nodes[b as usize].subtree_files),
    }
}

/// Signed logical-minus-allocated difference safe against u64 overflow
/// (returns the value as i128 for ordering; display uses the split form).
fn signed_difference(model: &DiskAnalysisModel, idx: u32) -> i128 {
    let node = &model.nodes[idx as usize];
    node.subtree_size as i128 - node.subtree_allocated_size as i128
}

#[cfg(test)]
mod tests {
    use super::*;
    use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

    fn record(
        frn: u64,
        parent_frn: u64,
        name: &str,
        size: u64,
        allocated_size: u64,
        is_dir: bool,
    ) -> DiskAnalysisRecord {
        DiskAnalysisRecord {
            frn,
            parent_frn,
            name: name.to_string(),
            size,
            allocated_size,
            is_dir,
            is_reparse: false,
        }
    }

    fn sorting_model() -> Arc<DiskAnalysisModel> {
        Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
            drive_letter: 'C',
            records: vec![
                record(5, 5, "", 0, 0, true),
                record(6, 5, "small", 0, 0, true),
                record(7, 6, "small.bin", 10, 10, false),
                record(8, 5, "large", 0, 0, true),
                record(9, 8, "large.bin", 20, 20, false),
                record(10, 5, "root.bin", 30, 30, false),
            ],
        }))
    }

    fn sorted_names(state: &DiskAnalysisState) -> Vec<String> {
        let model = state.model.as_ref().unwrap();
        state
            .largest_rows
            .iter()
            .map(|&idx| model.nodes[idx as usize].name.clone())
            .collect()
    }

    fn all_sorting_rows(model: &DiskAnalysisModel) -> Vec<u32> {
        (1..model.nodes.len() as u32)
            .filter(|&idx| idx != model.root)
            .collect()
    }

    #[test]
    fn reset_sort_uses_the_column_default_direction() {
        let mut state = DiskAnalysisState::new();

        state.reset_largest_sort(LargestColumn::Type);
        assert_eq!(
            state.largest_sort_criteria,
            [LargestSortCriterion {
                column: LargestColumn::Type,
                ascending: true,
            }]
        );

        state.reset_largest_sort(LargestColumn::Allocated);
        assert_eq!(
            state.largest_sort_criteria,
            [LargestSortCriterion {
                column: LargestColumn::Allocated,
                ascending: false,
            }]
        );
    }

    #[test]
    fn largest_sorting_rejects_a_third_criterion() {
        let mut state = DiskAnalysisState::new();
        let model = sorting_model();
        state.largest_rows = Arc::new(all_sorting_rows(&model));
        state.model = Some(model);

        state.update_largest_sort(LargestColumn::Type, false);
        state.update_largest_sort(LargestColumn::Allocated, true);
        let criteria = state.largest_sort_criteria.clone();
        let rows = state.largest_rows.clone();

        state.update_largest_sort(LargestColumn::Logical, true);

        assert_eq!(state.largest_sort_criteria, criteria);
        assert_eq!(state.largest_sort_criteria.len(), MAX_LARGEST_SORT_CRITERIA);
        assert_eq!(state.largest_rows, rows);
    }

    #[test]
    fn largest_sorting_supports_type_and_secondary_metric() {
        let mut state = DiskAnalysisState::new();
        let model = sorting_model();
        state.largest_rows = Arc::new(all_sorting_rows(&model));
        state.model = Some(model);

        state.update_largest_sort(LargestColumn::Type, false);
        assert_eq!(
            sorted_names(&state),
            vec!["small", "large", "small.bin", "large.bin", "root.bin"]
        );

        state.update_largest_sort(LargestColumn::Allocated, true);
        assert_eq!(
            state.largest_sort_criteria,
            [
                LargestSortCriterion {
                    column: LargestColumn::Type,
                    ascending: true,
                },
                LargestSortCriterion {
                    column: LargestColumn::Allocated,
                    ascending: false,
                },
            ]
        );
        assert_eq!(
            sorted_names(&state),
            vec!["large", "small", "root.bin", "large.bin", "small.bin"]
        );
    }

    #[test]
    fn largest_sorting_toggles_type_direction_and_replaces_secondary_criteria() {
        let mut state = DiskAnalysisState::new();
        let model = sorting_model();
        state.largest_rows = Arc::new(all_sorting_rows(&model));
        state.model = Some(model);

        state.update_largest_sort(LargestColumn::Type, false);
        state.update_largest_sort(LargestColumn::Allocated, true);
        state.update_largest_sort(LargestColumn::Allocated, true);
        assert_eq!(
            state.largest_sort_criteria,
            [
                LargestSortCriterion {
                    column: LargestColumn::Type,
                    ascending: true,
                },
                LargestSortCriterion {
                    column: LargestColumn::Allocated,
                    ascending: true,
                },
            ]
        );
        assert_eq!(
            sorted_names(&state),
            vec!["small", "large", "small.bin", "large.bin", "root.bin"]
        );

        state.update_largest_sort(LargestColumn::Type, false);
        assert_eq!(
            state.largest_sort_criteria,
            [LargestSortCriterion {
                column: LargestColumn::Type,
                ascending: false,
            }]
        );
        assert_eq!(
            sorted_names(&state),
            vec!["small.bin", "large.bin", "root.bin", "small", "large"]
        );
    }
}
