//! Bounded selection over the entire subtree using the table's actual ordering.

use super::{ActiveWeights, DiskAnalysisModel, SizeMetric};
use crate::app::disk_analysis_sorting::{
    compare_largest_rows, LargestColumn, LargestGrouping, LargestSort,
};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

#[derive(Clone, Copy)]
struct Candidate<'a> {
    idx: u32,
    model: &'a DiskAnalysisModel,
    sort: LargestSort,
    metric: SizeMetric,
    filtered: Option<&'a ActiveWeights>,
}

impl Ord for Candidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_largest_rows(
            self.model,
            self.idx,
            other.idx,
            self.sort,
            self.metric,
            self.filtered,
        )
    }
}
impl PartialOrd for Candidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Candidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Candidate<'_> {}

/// Select the first K rows under the requested sort, independently of the chart
/// metric. Grouped views (and file-count rankings) retain K of each item kind so
/// folders cannot consume the files' slots. No full-volume sort is performed.
pub fn largest_items(
    model: &DiskAnalysisModel,
    root: u32,
    sort: LargestSort,
    metric: SizeMetric,
    filtered: Option<&ActiveWeights>,
    k: usize,
    is_cancelled: impl Fn() -> bool,
) -> Vec<u32> {
    if k == 0 {
        return Vec::new();
    }
    let separate = sort.grouping != LargestGrouping::Mixed || sort.column == LargestColumn::Files;
    let mut primary = BinaryHeap::with_capacity(k.min(1024));
    let mut files = BinaryHeap::with_capacity(k.min(1024));
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if is_cancelled() {
            return Vec::new();
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir && !node.is_reparse {
            stack.extend_from_slice(model.children(idx));
        }
        if idx == root
            || filtered.is_some_and(|w| !w.matches.get(idx as usize).copied().unwrap_or(false))
        {
            continue;
        }
        let candidate = Candidate {
            idx,
            model,
            sort,
            metric,
            filtered,
        };
        let heap = if separate && !node.is_dir {
            &mut files
        } else {
            &mut primary
        };
        // The max-heap's root is the last (worst) retained row in table order.
        if heap.len() < k {
            heap.push(candidate);
        } else if heap.peek().is_some_and(|worst| candidate < *worst) {
            *heap.peek_mut().expect("nonempty bounded heap") = candidate;
        }
    }
    let mut rows: Vec<_> = primary.into_iter().chain(files).collect();
    rows.sort();
    rows.into_iter().map(|row| row.idx).collect()
}

#[cfg(test)]
mod tests;
