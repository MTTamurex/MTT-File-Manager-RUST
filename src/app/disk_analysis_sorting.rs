//! Independent grouping and column sorting for the analyzer's largest-items table.

use super::disk_analysis_model::DiskAnalysisModel;
use super::disk_analysis_query::{ActiveWeights, SizeMetric};
use super::disk_analysis_state::DiskAnalysisState;
use std::cmp::Ordering as SortOrdering;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LargestGrouping {
    #[default]
    Mixed,
    FoldersFirst,
    FilesFirst,
}

impl LargestGrouping {
    pub fn next(self) -> Self {
        match self {
            Self::Mixed => Self::FoldersFirst,
            Self::FoldersFirst => Self::FilesFirst,
            Self::FilesFirst => Self::Mixed,
        }
    }
}

/// Type is a header action for grouping, not a sortable value column.
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
    pub const SORTABLE: [Self; 6] = [
        Self::Name,
        Self::Path,
        Self::Logical,
        Self::Allocated,
        Self::Difference,
        Self::Files,
    ];

    pub const fn default_ascending(self) -> bool {
        matches!(self, Self::Name | Self::Path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargestSort {
    pub grouping: LargestGrouping,
    pub column: LargestColumn,
    pub ascending: bool,
}

impl Default for LargestSort {
    fn default() -> Self {
        Self {
            grouping: LargestGrouping::Mixed,
            column: LargestColumn::Allocated,
            ascending: false,
        }
    }
}

impl DiskAnalysisState {
    /// Metric changes reset the value ordering, but preserve grouping.
    pub fn reset_largest_sort(&mut self, column: LargestColumn) {
        debug_assert_ne!(column, LargestColumn::Type);
        self.largest_sort.column = column;
        self.largest_sort.ascending = column.default_ascending();
    }

    /// Header clicks change one independent preference at a time.
    pub fn update_largest_sort(&mut self, column: LargestColumn) {
        if column == LargestColumn::Type {
            self.largest_sort.grouping = self.largest_sort.grouping.next();
        } else if self.largest_sort.column == column {
            self.largest_sort.ascending = !self.largest_sort.ascending;
        } else {
            self.reset_largest_sort(column);
        }
        self.sort_largest_rows();
    }

    pub fn sort_largest_rows(&mut self) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let sort = self.largest_sort;
        let filtered = self.active_weights.clone();
        let mut rows = self.largest_rows.as_ref().clone();
        rows.sort_by(|&a, &b| {
            let type_order = model.nodes[b as usize]
                .is_dir
                .cmp(&model.nodes[a as usize].is_dir);
            let group_order = match sort.grouping {
                LargestGrouping::Mixed => SortOrdering::Equal,
                LargestGrouping::FoldersFirst => type_order,
                LargestGrouping::FilesFirst => type_order.reverse(),
            };
            group_order.then_with(|| {
                let order = compare_largest_column(
                    &model,
                    a,
                    b,
                    sort.column,
                    self.metric,
                    filtered.as_ref(),
                );
                let order = if sort.ascending {
                    order
                } else {
                    order.reverse()
                };
                // Individual files all have count 1; retain useful size ranking.
                order
                    .then_with(|| {
                        if sort.column == LargestColumn::Files {
                            model.nodes[b as usize]
                                .subtree_size
                                .cmp(&model.nodes[a as usize].subtree_size)
                        } else {
                            SortOrdering::Equal
                        }
                    })
                    .then_with(|| a.cmp(&b))
            })
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
        LargestColumn::Type => SortOrdering::Equal,
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

/// i128 holds the signed difference of any two u64 sizes without overflow.
fn signed_difference(model: &DiskAnalysisModel, idx: u32) -> i128 {
    let node = &model.nodes[idx as usize];
    node.subtree_size as i128 - node.subtree_allocated_size as i128
}

#[cfg(test)]
mod tests;
