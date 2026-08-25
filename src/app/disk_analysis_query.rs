//! Query engine shared by the disk analyzer views: size metrics, combined
//! filters, largest-items top-K, subtree search and logical-vs-allocated
//! efficiency analysis.
//!
//! Everything here is a pure function over [`DiskAnalysisModel`] so it can
//! run off the UI thread (worker jobs) and be tested without egui. Traversals
//! start at a subtree root, never descend through reparse points and simply
//! ignore unreachable components (cycles/orphans outside the walked tree).

use crate::app::disk_analysis_model::{AnalysisNode, DiskAnalysisModel};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::Arc;

/// Which quantity drives the treemap, tables and charts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SizeMetric {
    /// Physical allocation currently used by the analyzer (original view).
    #[default]
    Allocated,
    /// Logical file size (bytes declared by the directory entry).
    Logical,
    /// One unit per file; directories weigh their descendant file count.
    FileCount,
}

impl SizeMetric {
    pub const ALL: [SizeMetric; 3] = [
        SizeMetric::Allocated,
        SizeMetric::Logical,
        SizeMetric::FileCount,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    /// Value of a single node ignoring its descendants.
    pub fn own(self, node: &AnalysisNode) -> u64 {
        match self {
            SizeMetric::Allocated => node.allocated_size,
            SizeMetric::Logical => node.size,
            SizeMetric::FileCount => u64::from(!node.is_dir),
        }
    }

    /// Aggregated value over the node's subtree (reparse points do not
    /// descend, matching how the model aggregates).
    pub fn subtree(self, node: &AnalysisNode) -> u64 {
        match self {
            SizeMetric::Allocated => node.subtree_allocated_size,
            SizeMetric::Logical => node.subtree_size,
            SizeMetric::FileCount => node.subtree_files,
        }
    }
}

/// Size basis used by the min/max size filter conditions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FilterSizeBase {
    #[default]
    Logical,
    Allocated,
}

/// Combined filter evaluated on files; directories stay visible while they
/// contain at least one matching descendant.
///
/// Extension entries are compared case-insensitively and may carry a leading
/// dot (normalized by [`DiskAnalysisFilter::parse_extensions`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskAnalysisFilter {
    /// Bit `i` set keeps `FileCategory::ALL[i]`; all set means "any".
    pub categories_mask: u8,
    pub extensions: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub size_base: FilterSizeBase,
}

impl Default for DiskAnalysisFilter {
    fn default() -> Self {
        Self {
            categories_mask: Self::all_categories_mask(),
            extensions: Vec::new(),
            min_size: None,
            max_size: None,
            size_base: FilterSizeBase::default(),
        }
    }
}

impl DiskAnalysisFilter {
    pub fn all_categories_mask() -> u8 {
        0xFF
    }

    /// Parses raw user input ("pdf, .PNG txt") into normalized lowercase
    /// extensions without the leading dot. Empty segments are dropped.
    pub fn parse_extensions(input: &str) -> Vec<String> {
        let mut out = Vec::new();
        for token in input.split([',', ';', ' ']) {
            let token = token.trim().trim_start_matches('.').to_ascii_lowercase();
            if !token.is_empty() && !out.contains(&token) {
                out.push(token);
            }
        }
        out
    }

    /// True when any condition is restrictive (i.e. the filter can hide data).
    pub fn is_active(&self) -> bool {
        self.categories_mask != Self::all_categories_mask()
            || !self.extensions.is_empty()
            || self.min_size.is_some()
            || self.max_size.is_some()
    }

    fn size_in_range(&self, node: &AnalysisNode) -> bool {
        let value = match self.size_base {
            FilterSizeBase::Logical => node.size,
            FilterSizeBase::Allocated => node.allocated_size,
        };
        if let Some(min) = self.min_size {
            if value < min {
                return false;
            }
        }
        if let Some(max) = self.max_size {
            if value > max {
                return false;
            }
        }
        true
    }

    /// Whether one file (or leaf-like reparse point) satisfies every set
    /// condition. Directories are never decided here.
    pub fn matches_leaf(&self, node: &AnalysisNode) -> bool {
        debug_assert!(!node.is_dir || node.is_reparse);
        if self.categories_mask != Self::all_categories_mask()
            && self.categories_mask & (1 << node.category.index()) == 0
        {
            return false;
        }
        if !self.extensions.is_empty() {
            let ext = match node.name.rsplit_once('.') {
                Some((_, e)) => e.to_ascii_lowercase(),
                None => String::new(),
            };
            if !self.extensions.contains(&ext) {
                return false;
            }
        }
        self.size_in_range(node)
    }
}

/// Per-node filtered weights aligned with `model.nodes` (index -> weight).
///
/// Produced by [`compute_filtered_weights`]: only nodes reachable from the
/// walked root receive a value; directories roll up the matching descendants
/// so the treemap can resize purely from this vector.
#[derive(Default)]
pub struct FilteredWeights {
    pub root: u32,
    pub weights: Vec<u64>,
    /// Membership is independent of weight so zero-byte matched files remain
    /// searchable and eligible for efficiency analysis.
    pub matches: Vec<bool>,
}

impl FilteredWeights {
    pub fn total_at_root(&self) -> u64 {
        self.weights.get(self.root as usize).copied().unwrap_or(0)
    }
}

/// Iterative post-order walk from `root` producing filtered weights.
///
/// - Files (and reparse leaves) contribute the selected metric when they match
///   every active condition. The size base is used only by min/max matching.
/// - Directories contribute the sum of matching descendants (never their own
///   aggregate size: the size condition applies per file).
/// - Reparse directories are not descended into.
pub fn compute_filtered_weights(
    model: &DiskAnalysisModel,
    root: u32,
    filter: &DiskAnalysisFilter,
    metric: SizeMetric,
) -> FilteredWeights {
    compute_filtered_weights_cancellable(model, root, filter, metric, || false)
        .expect("non-cancellable filter computation")
}

pub fn compute_filtered_weights_cancellable(
    model: &DiskAnalysisModel,
    root: u32,
    filter: &DiskAnalysisFilter,
    metric: SizeMetric,
    is_cancelled: impl Fn() -> bool,
) -> Option<FilteredWeights> {
    let mut out = FilteredWeights {
        root,
        weights: vec![0; model.nodes.len()],
        matches: vec![false; model.nodes.len()],
    };
    if !filter.is_active() {
        collect_subtree_values(model, root, metric, &mut out.weights, &mut out.matches);
        return Some(out);
    }

    // (idx, visited): post-order so parents sum finished children.
    let mut stack: Vec<(u32, bool)> = vec![(root, false)];
    while let Some((idx, visited)) = stack.pop() {
        if is_cancelled() {
            return None;
        }
        let node = &model.nodes[idx as usize];
        if visited {
            if node.is_dir && !node.is_reparse {
                let mut total = 0u64;
                let mut any_match = false;
                for &child in model.children(idx) {
                    total = total.saturating_add(out.weights[child as usize]);
                    any_match |= out.matches[child as usize];
                }
                out.weights[idx as usize] = total;
                out.matches[idx as usize] = any_match;
            } else {
                let matched = filter.matches_leaf(node);
                out.matches[idx as usize] = matched;
                if matched {
                    out.weights[idx as usize] = metric.own(node);
                }
            }
            continue;
        }
        stack.push((idx, true));
        if node.is_dir && !node.is_reparse {
            for &child in model.children(idx) {
                stack.push((child, false));
            }
        }
    }
    Some(out)
}

fn collect_subtree_values(
    model: &DiskAnalysisModel,
    root: u32,
    metric: SizeMetric,
    weights: &mut [u64],
    matches: &mut [bool],
) {
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        let node = &model.nodes[idx as usize];
        weights[idx as usize] = metric.subtree(node);
        matches[idx as usize] = true;
        if node.is_dir && !node.is_reparse {
            for &child in model.children(idx) {
                stack.push(child);
            }
        }
    }
}

/// Weight basis for top-K / search inclusion checks.
#[derive(Clone, Copy)]
pub enum WeightBasis<'a> {
    /// Plain metric read from the model.
    Metric(SizeMetric),
    /// Precomputed filtered weights; zero hides the node.
    Filtered(&'a [u64]),
}

/// Filtered per-node weights currently attached to the treemap/list views.
/// `id` identifies this snapshot in cache keys; `root` records which subtree
/// produced it.
#[derive(Clone)]
pub struct ActiveWeights {
    pub id: u64,
    pub root: u32,
    pub metric: SizeMetric,
    pub weights: Arc<Vec<u64>>,
    pub matches: Arc<Vec<bool>>,
}

impl WeightBasis<'_> {
    fn subtree_value(self, model: &DiskAnalysisModel, idx: u32) -> u64 {
        match self {
            WeightBasis::Metric(m) => m.subtree(&model.nodes[idx as usize]),
            WeightBasis::Filtered(w) => w.get(idx as usize).copied().unwrap_or(0),
        }
    }
    fn includes(self, model: &DiskAnalysisModel, idx: u32) -> bool {
        self.subtree_value(model, idx) > 0
    }
}

/// One projected row of the "Largest items" table: just the node index; all
/// displayed numbers are read straight from the model at paint time.
pub type LargestProjection = Vec<u32>;

/// Top-K heaviest descendants of `root` (root itself excluded), computed with
/// a bounded heap so millions of nodes never get sorted. Ties break by lower
/// index first, making the output deterministic.
pub fn largest_items(
    model: &DiskAnalysisModel,
    root: u32,
    basis: WeightBasis<'_>,
    k: usize,
    is_cancelled: impl Fn() -> bool,
) -> LargestProjection {
    if k == 0 {
        return Vec::new();
    }
    // The heap root is the worst retained entry: lowest weight and, on a
    // tie, highest index. This preserves the documented lower-index tie-break.
    let mut heap: BinaryHeap<Reverse<(u64, Reverse<u32>)>> = BinaryHeap::with_capacity(k.min(1024));
    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if is_cancelled() {
            return Vec::new();
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir && !node.is_reparse {
            for &child in model.children(idx) {
                stack.push(child);
            }
        }
        if idx == root || !basis.includes(model, idx) {
            continue;
        }
        let weight = basis.subtree_value(model, idx);
        // Bounded min-heap of the k heaviest entries so far: peek yields the
        // smallest kept tuple; a new entry replaces it only when heavier.
        let key = (weight, Reverse(idx));
        if heap.len() < k {
            heap.push(Reverse(key));
        } else if let Some(&Reverse(smallest)) = heap.peek() {
            if key > smallest {
                heap.pop();
                heap.push(Reverse(key));
            }
        }
    }
    let mut rows: Vec<(u64, u32)> = heap
        .into_iter()
        .map(|Reverse((weight, Reverse(idx)))| (weight, idx))
        .collect();
    // Heaviest first; deterministic tie-break by ascending index.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    rows.into_iter().map(|(_, idx)| idx).collect()
}

/// One efficiency finding: a file whose logical size and physical allocation
/// diverge. The sign lives in the group membership, never in arithmetic on
/// unsigned sizes (no u64 subtraction without a guard).
#[derive(Clone, Copy, Debug)]
pub struct EfficiencyRow {
    pub idx: u32,
    pub logical: u64,
    pub allocated: u64,
    pub absolute_difference: u64,
}

#[derive(Default, Clone, Debug)]
pub struct EfficiencyResult {
    /// Sparse/compressed/resident-like files: logical > allocated.
    pub logical_greater: Vec<EfficiencyRow>,
    /// Cluster slack / ADS-like files: allocated > logical.
    pub allocated_greater: Vec<EfficiencyRow>,
    pub logical_greater_total: u64,
    pub allocated_greater_total: u64,
    /// True when either list was cut at its limit.
    pub truncated: bool,
}

/// Scan `root`'s files for logical-vs-allocated divergence, keeping the
/// biggest absolute differences per group (bounded heaps) plus full-subtree
/// totals. Directories are ignored; filters apply via `matches`.
pub fn efficiency_scan(
    model: &DiskAnalysisModel,
    root: u32,
    matches: Option<&[bool]>,
    limit_per_group: usize,
    is_cancelled: impl Fn() -> bool,
) -> EfficiencyResult {
    let mut result = EfficiencyResult::default();
    let mut logical_heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();
    let mut allocated_heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();

    let mut stack = vec![root];
    while let Some(idx) = stack.pop() {
        if is_cancelled() {
            result.truncated = true;
            return result;
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir {
            if !node.is_reparse {
                for &child in model.children(idx) {
                    stack.push(child);
                }
            }
            continue;
        }
        if !matches.is_none_or(|m| m.get(idx as usize).copied().unwrap_or(false)) {
            continue;
        }
        let (logical, allocated) = (node.size, node.allocated_size);
        let diff = match logical.abs_diff(allocated) {
            0 => continue,
            d => d,
        };
        if logical > allocated {
            result.logical_greater_total = result.logical_greater_total.saturating_add(diff);
            push_bounded(
                &mut logical_heap,
                (diff, idx),
                limit_per_group,
                &mut result.truncated,
            );
        } else {
            result.allocated_greater_total = result.allocated_greater_total.saturating_add(diff);
            push_bounded(
                &mut allocated_heap,
                (diff, idx),
                limit_per_group,
                &mut result.truncated,
            );
        }
    }

    result.logical_greater = drain_sorted(model, &logical_heap);
    result.allocated_greater = drain_sorted(model, &allocated_heap);
    result
}

fn push_bounded(
    heap: &mut BinaryHeap<Reverse<(u64, u32)>>,
    entry: (u64, u32),
    limit: usize,
    truncated: &mut bool,
) {
    if limit == 0 {
        *truncated = true;
        return;
    }
    if heap.len() < limit {
        heap.push(Reverse(entry));
    } else if let Some(&Reverse(smallest)) = heap.peek() {
        if entry > smallest {
            *truncated = true;
            heap.pop();
            heap.push(Reverse(entry));
        } else {
            *truncated = true;
        }
    }
}

fn drain_sorted(
    model: &DiskAnalysisModel,
    heap: &BinaryHeap<Reverse<(u64, u32)>>,
) -> Vec<EfficiencyRow> {
    let mut rows: Vec<(u64, u32)> = heap.iter().map(|Reverse(v)| *v).collect();
    // Heaviest difference first; deterministic tie-break by ascending index.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    rows.into_iter()
        .map(|(diff, idx)| {
            let node = &model.nodes[idx as usize];
            EfficiencyRow {
                idx,
                logical: node.size,
                allocated: node.allocated_size,
                absolute_difference: diff,
            }
        })
        .collect()
}

/// Case-insensitive subtree search by name, or by full path when the query
/// contains a path separator. Returns up to `limit` indices in traversal
/// (record) order, so the output is deterministic. Filters apply via `matches`.
pub fn search_nodes(
    model: &DiskAnalysisModel,
    root: u32,
    query: &str,
    matches: Option<&[bool]>,
    limit: usize,
    is_cancelled: impl Fn() -> bool,
) -> Vec<u32> {
    let query = query.trim();
    let needle = query.to_lowercase();
    if needle.is_empty() || limit == 0 {
        return Vec::new();
    }
    let path_mode = needle.contains(['\\', '/']);
    let path_needle = needle.replace('/', "\\");

    let mut hits = Vec::new();
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(idx) = queue.pop_front() {
        if is_cancelled() {
            return hits;
        }
        let node = &model.nodes[idx as usize];
        if node.is_dir && !node.is_reparse {
            for &child in model.children(idx) {
                queue.push_back(child);
            }
        }
        if idx == root || !matches.is_none_or(|m| m.get(idx as usize).copied().unwrap_or(false)) {
            continue;
        }
        let matched = if path_mode {
            model.path_of(idx).to_lowercase().contains(&path_needle)
        } else {
            node.name.to_lowercase().contains(&needle)
        };
        if matched {
            hits.push(idx);
            if hits.len() >= limit {
                break;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::disk_analysis_model::FileCategory;
    use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

    fn rec(
        frn: u64,
        parent_frn: u64,
        name: &str,
        size: u64,
        allocated: u64,
        is_dir: bool,
    ) -> DiskAnalysisRecord {
        DiskAnalysisRecord {
            frn,
            parent_frn,
            name: name.to_string(),
            size,
            allocated_size: allocated,
            is_dir,
            is_reparse: false,
        }
    }

    fn build(records: Vec<DiskAnalysisRecord>) -> DiskAnalysisModel {
        DiskAnalysisModel::build(DiskAnalysisSnapshot {
            drive_letter: 'C',
            records,
        })
    }

    fn find(model: &DiskAnalysisModel, name: &str) -> u32 {
        model
            .nodes
            .iter()
            .position(|n| n.name == name)
            .expect("node exists") as u32
    }

    fn sample_model() -> DiskAnalysisModel {
        build(vec![
            rec(5, 5, "", 0, 0, true),      // volume root
            rec(10, 5, "docs", 0, 0, true), // dir
            rec(11, 10, "report.pdf", 1000, 4096, false),
            rec(12, 10, "notes.TXT", 500, 512, false),
            rec(13, 5, "movie.MP4", 900_000, 100_000, false),
            rec(14, 999, "orphan.bin", 7, 7, false),
        ])
    }

    #[test]
    fn metric_values_logical_allocated_and_count() {
        let model = sample_model();
        let docs = find(&model, "docs");
        let dir = &model.nodes[docs as usize];

        assert_eq!(SizeMetric::Logical.subtree(dir), 1500);
        assert_eq!(SizeMetric::Allocated.subtree(dir), 4608);
        assert_eq!(SizeMetric::FileCount.subtree(dir), 2);

        let movie = find(&model, "movie.MP4");
        let file = &model.nodes[movie as usize];
        assert_eq!(SizeMetric::Logical.own(file), 900_000);
        assert_eq!(SizeMetric::Allocated.own(file), 100_000);
        assert_eq!(SizeMetric::FileCount.own(file), 1);
    }

    #[test]
    fn combined_filters_evaluate_files_only() {
        let model = sample_model();
        let docs = find(&model, "docs");
        let pdf = find(&model, "report.pdf");

        let mut filter = DiskAnalysisFilter {
            categories_mask: DiskAnalysisFilter::all_categories_mask(),
            extensions: vec!["pdf".to_string()],
            min_size: Some(2000), // allocated of report.pdf is 4096
            max_size: None,
            size_base: FilterSizeBase::Allocated,
        };
        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Allocated);
        assert_eq!(weights.weights[pdf as usize], 4096);
        assert_eq!(weights.weights[docs as usize], 4096); // rollup
        let txt = find(&model, "notes.TXT");
        assert_eq!(weights.weights[txt as usize], 0);

        // Logical base fails the same min size.
        filter.size_base = FilterSizeBase::Logical;
        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Allocated);
        assert_eq!(weights.weights[pdf as usize], 0);
        assert_eq!(weights.weights[docs as usize], 0);

        // Max size only.
        let filter = DiskAnalysisFilter {
            categories_mask: DiskAnalysisFilter::all_categories_mask(),
            extensions: Vec::new(),
            min_size: None,
            max_size: Some(600),
            size_base: FilterSizeBase::Logical,
        };
        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Logical);
        assert_eq!(weights.weights[find(&model, "notes.TXT") as usize], 500);
        assert_eq!(weights.weights[find(&model, "movie.MP4") as usize], 0);
        // Orphan is reachable from the tree root and matches too.
        assert_eq!(weights.weights[find(&model, "orphan.bin") as usize], 7);
    }

    #[test]
    fn extensions_are_case_insensitive_with_or_without_dot() {
        let parsed = DiskAnalysisFilter::parse_extensions(" .MP4, txt;;PdF ");
        assert_eq!(parsed, vec!["mp4", "txt", "pdf"]);

        let model = sample_model();
        let movie = find(&model, "movie.MP4");
        let node = &model.nodes[movie as usize];
        let filter = DiskAnalysisFilter {
            categories_mask: DiskAnalysisFilter::all_categories_mask(),
            extensions: vec!["mp4".to_string()],
            min_size: None,
            max_size: None,
            size_base: FilterSizeBase::Logical,
        };
        assert!(filter.matches_leaf(node));
        let notes = find(&model, "notes.TXT");
        assert!(!filter.matches_leaf(&model.nodes[notes as usize]));
    }

    #[test]
    fn category_mask_filters_by_file_category() {
        let model = sample_model();
        let video_only = 1 << FileCategory::Video.index();
        let filter = DiskAnalysisFilter {
            categories_mask: video_only,
            ..DiskAnalysisFilter::default()
        };
        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Logical);
        assert_eq!(weights.weights[find(&model, "movie.MP4") as usize], 900_000);
        assert_eq!(weights.weights[find(&model, "report.pdf") as usize], 0);
    }

    #[test]
    fn inactive_filter_returns_natural_rollups() {
        let model = sample_model();
        let filter = DiskAnalysisFilter::default();
        assert!(!filter.is_active());
        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Allocated);
        let docs = find(&model, "docs");
        assert_eq!(
            weights.weights[docs as usize],
            model.nodes[docs as usize].subtree_allocated_size
        );
    }

    #[test]
    fn filtered_weights_follow_selected_metric_not_size_filter_base() {
        let model = sample_model();
        let docs = find(&model, "docs");
        let pdf = find(&model, "report.pdf");
        let filter = DiskAnalysisFilter {
            categories_mask: DiskAnalysisFilter::all_categories_mask(),
            extensions: vec!["pdf".to_string()],
            min_size: Some(4_000),
            max_size: None,
            size_base: FilterSizeBase::Allocated,
        };

        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::FileCount);

        assert_eq!(weights.weights[pdf as usize], 1);
        assert_eq!(weights.weights[docs as usize], 1);
        assert!(weights.matches[pdf as usize]);
    }

    #[test]
    fn zero_weight_matches_remain_searchable_and_visible_to_efficiency() {
        let model = build(vec![
            rec(5, 5, "", 0, 0, true),
            rec(10, 5, "resident.bin", 100, 0, false),
        ]);
        let file = find(&model, "resident.bin");
        let filter = DiskAnalysisFilter {
            categories_mask: DiskAnalysisFilter::all_categories_mask(),
            extensions: vec!["bin".to_string()],
            min_size: None,
            max_size: None,
            size_base: FilterSizeBase::Logical,
        };
        let filtered = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Allocated);

        assert_eq!(filtered.weights[file as usize], 0);
        assert!(filtered.matches[file as usize]);
        assert_eq!(
            search_nodes(
                &model,
                model.root,
                "resident",
                Some(&filtered.matches),
                10,
                || false,
            ),
            vec![file]
        );
        let efficiency = efficiency_scan(&model, model.root, Some(&filtered.matches), 10, || false);
        assert_eq!(efficiency.logical_greater[0].idx, file);
    }

    #[test]
    fn reparse_points_are_not_descended() {
        let mut junction = rec(30, 5, "junction", 0, 16, true);
        junction.is_reparse = true;
        let model = build(vec![
            rec(5, 5, "", 0, 0, true),
            junction,
            rec(31, 30, "inside.mp4", 9_000, 9_000, false),
            rec(32, 5, "outside.mp4", 1_000, 1_000, false),
        ]);
        let filter = DiskAnalysisFilter {
            categories_mask: DiskAnalysisFilter::all_categories_mask(),
            extensions: vec!["mp4".to_string()],
            min_size: None,
            max_size: None,
            size_base: FilterSizeBase::Logical,
        };
        let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Logical);
        let outside = find(&model, "outside.mp4");
        assert_eq!(weights.weights[outside as usize], 1_000);
        // inside.mp4 must not be reached through the junction.
        let inside = find(&model, "inside.mp4");
        assert_eq!(weights.weights[inside as usize], 0);
        assert_eq!(weights.total_at_root(), 1_000);

        // Same guarantee for top-K and search.
        let rows = largest_items(
            &model,
            model.root,
            WeightBasis::Metric(SizeMetric::Logical),
            10,
            || false,
        );
        assert!(!rows.contains(&inside));
        let hits = search_nodes(&model, model.root, "inside", None, 50, || false);
        assert!(hits.is_empty());
    }

    #[test]
    fn top_k_covers_requested_subtree_and_excludes_root() {
        let model = sample_model();
        let docs = find(&model, "docs");
        let rows = largest_items(
            &model,
            docs,
            WeightBasis::Metric(SizeMetric::Logical),
            10,
            || false,
        );
        let names: Vec<&str> = rows
            .iter()
            .map(|&i| model.nodes[i as usize].name.as_str())
            .collect();
        assert_eq!(names, vec!["report.pdf", "notes.TXT"]);
        assert!(!rows.contains(&docs));

        // K bounds the output; heaviest first across the whole drive.
        // movie.MP4 allocates 100 KB; the docs folder rolls up 4608 B,
        // edging out its own child report.pdf (4096 B).
        let rows = largest_items(
            &model,
            model.root,
            WeightBasis::Metric(SizeMetric::Allocated),
            2,
            || false,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(model.nodes[rows[0] as usize].name, "movie.MP4");
        assert_eq!(model.nodes[rows[1] as usize].name, "docs");
    }

    #[test]
    fn top_k_is_deterministic_on_ties() {
        let model = build(vec![
            rec(5, 5, "", 0, 0, true),
            rec(10, 5, "b.bin", 100, 100, false),
            rec(11, 5, "a.bin", 100, 100, false),
        ]);
        let first = largest_items(
            &model,
            model.root,
            WeightBasis::Metric(SizeMetric::Logical),
            10,
            || false,
        );
        let second = largest_items(
            &model,
            model.root,
            WeightBasis::Metric(SizeMetric::Logical),
            10,
            || false,
        );
        assert_eq!(first, second);
        assert_eq!(first.len(), 2);

        let one = largest_items(
            &model,
            model.root,
            WeightBasis::Metric(SizeMetric::Logical),
            1,
            || false,
        );
        assert_eq!(one, vec![first[0].min(first[1])]);
    }

    #[test]
    fn efficiency_scan_splits_groups_and_totals_without_overflow() {
        let sparse = rec(11, 5, "sparse.bin", 1 << 62, 8_192, false);
        let model = build(vec![
            rec(5, 5, "", 0, 0, true),
            sparse,
            rec(12, 5, "slack.bin", 10, 1 << 40, false),
            rec(13, 5, "even.bin", 100, 100, false),
        ]);
        let result = efficiency_scan(&model, model.root, None, 100, || false);
        assert_eq!(result.logical_greater.len(), 1);
        assert_eq!(
            result.logical_greater[0].absolute_difference,
            (1u64 << 62) - 8_192
        );
        assert_eq!(result.logical_greater_total, (1u64 << 62) - 8_192);
        assert_eq!(result.allocated_greater.len(), 1);
        assert_eq!(
            result.allocated_greater[0].absolute_difference,
            (1u64 << 40) - 10
        );
        // even.bin appears in neither group.
        assert!(result.logical_greater_total > 0);
    }

    #[test]
    fn efficiency_limit_marks_truncation_but_keeps_totals() {
        let records: Vec<DiskAnalysisRecord> = (0..20)
            .map(|i| rec(10 + i, 5, &format!("f{i}.bin"), i * 100, i, false))
            .chain(vec![rec(5, 5, "", 0, 0, true)])
            .collect();
        let model = build(records);
        let result = efficiency_scan(&model, model.root, None, 5, || false);
        assert_eq!(result.logical_greater.len(), 5);
        assert!(result.truncated);
        // Total covers all 19 divergent files regardless of the row limit:
        // diff(i) = i*100 - i for i in 1..20 (f0 has zero difference).
        let expected: u64 = (1..20u64).map(|i| i * 100 - i).sum();
        assert_eq!(result.logical_greater_total, expected);
        assert_eq!(result.allocated_greater_total, 0);
    }

    #[test]
    fn search_matches_names_case_insensitively_and_paths_with_separator() {
        let model = sample_model();

        let hits = search_nodes(&model, model.root, "report", None, 200, || false);
        assert_eq!(hits, vec![find(&model, "report.pdf")]);

        // Case-insensitive name match.
        let hits = search_nodes(&model, model.root, "MOVIE", None, 200, || false);
        assert_eq!(hits, vec![find(&model, "movie.MP4")]);

        // Path mode requires a separator in the query.
        let hits = search_nodes(&model, model.root, r"\docs\", None, 200, || false);
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&find(&model, "report.pdf")));

        let hits = search_nodes(&model, model.root, "docs/report.pdf", None, 200, || false);
        assert_eq!(hits, vec![find(&model, "report.pdf")]);

        // Empty / whitespace-only queries yield nothing.
        assert!(search_nodes(&model, model.root, "   ", None, 200, || false,).is_empty());

        // Limit truncates deterministically in traversal order.
        let hits = search_nodes(&model, model.root, ".", None, 2, || false);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn search_respects_subtree_scope() {
        let model = sample_model();
        let docs = find(&model, "docs");
        let hits = search_nodes(&model, docs, "movie", None, 200, || false);
        assert!(hits.is_empty());
    }

    #[test]
    fn cancellation_yields_partial_empty_results() {
        let model = sample_model();
        assert!(largest_items(
            &model,
            model.root,
            WeightBasis::Metric(SizeMetric::Logical),
            10,
            || true,
        )
        .is_empty());
        assert!(efficiency_scan(&model, model.root, None, 10, || true,)
            .logical_greater
            .is_empty());
        assert!(compute_filtered_weights_cancellable(
            &model,
            model.root,
            &DiskAnalysisFilter {
                extensions: vec!["bin".to_string()],
                ..DiskAnalysisFilter::default()
            },
            SizeMetric::Logical,
            || true,
        )
        .is_none());
    }
}
