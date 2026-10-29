use super::*;
use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

fn state() -> DiskAnalysisState {
    let record = |frn, parent_frn, name: &str, size, allocated_size, is_dir| DiskAnalysisRecord {
        frn,
        parent_frn,
        name: name.to_string(),
        size,
        allocated_size,
        is_dir,
        is_reparse: false,
    };
    let model = Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'C',
        records: vec![
            record(5, 5, "", 0, 0, true),
            record(6, 5, "small", 0, 0, true),
            record(7, 6, "small.bin", 10, 10, false),
            record(8, 5, "large", 0, 0, true),
            record(9, 8, "large.bin", 20, 20, false),
            record(10, 5, "root.bin", 30, 30, false),
        ],
    }));
    let mut state = DiskAnalysisState::new();
    state.largest_rows = Arc::new(
        (1..model.nodes.len() as u32)
            .filter(|&i| i != model.root)
            .collect(),
    );
    state.model = Some(model);
    state
}

fn names(state: &DiskAnalysisState) -> Vec<&str> {
    let model = state.model.as_ref().unwrap();
    state
        .largest_rows
        .iter()
        .map(|&i| model.nodes[i as usize].name.as_str())
        .collect()
}

#[test]
fn default_is_mixed_allocated_descending() {
    assert_eq!(
        LargestSort::default(),
        LargestSort {
            grouping: LargestGrouping::Mixed,
            column: LargestColumn::Allocated,
            ascending: false,
        }
    );
}

#[test]
fn grouping_and_size_direction_are_independent() {
    for column in [LargestColumn::Logical, LargestColumn::Allocated] {
        for (grouping, ascending, expected) in [
            (
                LargestGrouping::FoldersFirst,
                false,
                vec!["large", "small", "root.bin", "large.bin", "small.bin"],
            ),
            (
                LargestGrouping::FoldersFirst,
                true,
                vec!["small", "large", "small.bin", "large.bin", "root.bin"],
            ),
            (
                LargestGrouping::FilesFirst,
                false,
                vec!["root.bin", "large.bin", "small.bin", "large", "small"],
            ),
            (
                LargestGrouping::FilesFirst,
                true,
                vec!["small.bin", "large.bin", "root.bin", "small", "large"],
            ),
            (
                LargestGrouping::Mixed,
                false,
                vec!["root.bin", "large", "large.bin", "small", "small.bin"],
            ),
            (
                LargestGrouping::Mixed,
                true,
                vec!["small", "small.bin", "large", "large.bin", "root.bin"],
            ),
        ] {
            let mut state = state();
            state.largest_sort = LargestSort {
                grouping,
                column,
                ascending,
            };
            state.sort_largest_rows();
            assert_eq!(
                names(&state),
                expected,
                "{grouping:?} {column:?} {ascending}"
            );
        }
    }
}

#[test]
fn headers_preserve_grouping_and_toggle_column_direction() {
    let mut state = state();
    state.update_largest_sort(LargestColumn::Type);
    assert_eq!(state.largest_sort.grouping, LargestGrouping::FoldersFirst);
    state.update_largest_sort(LargestColumn::Allocated);
    assert!(state.largest_sort.ascending);
    assert_eq!(state.largest_sort.grouping, LargestGrouping::FoldersFirst);
    state.update_largest_sort(LargestColumn::Logical);
    assert!(!state.largest_sort.ascending);
    state.update_largest_sort(LargestColumn::Type);
    assert_eq!(state.largest_sort.grouping, LargestGrouping::FilesFirst);
    assert_eq!(state.largest_sort.column, LargestColumn::Logical);
    assert!(!state.largest_sort.ascending);
    state.update_largest_sort(LargestColumn::Type);
    assert_eq!(state.largest_sort.grouping, LargestGrouping::Mixed);
}

#[test]
fn metric_reset_preserves_grouping_and_sets_default_direction() {
    let mut state = state();
    state.largest_sort.grouping = LargestGrouping::FilesFirst;
    for column in LargestColumn::SORTABLE {
        state.reset_largest_sort(column);
        assert_eq!(state.largest_sort.grouping, LargestGrouping::FilesFirst);
        assert_eq!(state.largest_sort.column, column);
        assert_eq!(
            state.largest_sort.ascending,
            matches!(column, LargestColumn::Name | LargestColumn::Path)
        );
    }
}

#[test]
fn name_sorting_works_inside_groups_in_both_directions() {
    let mut state = state();
    state.largest_sort.grouping = LargestGrouping::FoldersFirst;
    state.update_largest_sort(LargestColumn::Name);
    assert_eq!(
        names(&state),
        ["large", "small", "large.bin", "root.bin", "small.bin"]
    );
    state.update_largest_sort(LargestColumn::Name);
    assert_eq!(
        names(&state),
        ["small", "large", "small.bin", "root.bin", "large.bin"]
    );
}

#[test]
fn file_count_uses_size_to_break_ties() {
    let mut state = state();
    state.metric = SizeMetric::FileCount;
    state.largest_sort.grouping = LargestGrouping::FoldersFirst;
    state.reset_largest_sort(LargestColumn::Files);
    state.sort_largest_rows();
    assert_eq!(
        names(&state),
        ["large", "small", "root.bin", "large.bin", "small.bin"]
    );
}

#[test]
fn filtered_values_are_used_only_for_the_active_metric() {
    for (metric, column) in [
        (SizeMetric::Allocated, LargestColumn::Allocated),
        (SizeMetric::Logical, LargestColumn::Logical),
        (SizeMetric::FileCount, LargestColumn::Files),
    ] {
        let mut state = state();
        let model = state.model.as_ref().unwrap();
        let mut weights = vec![0; model.nodes.len()];
        for (i, node) in model.nodes.iter().enumerate() {
            weights[i] = if node.name.starts_with("small") {
                100
            } else {
                1
            };
        }
        state.active_weights = Some(ActiveWeights {
            id: 1,
            root: model.root,
            metric,
            weights: Arc::new(weights),
            matches: Arc::new(vec![true; model.nodes.len()]),
        });
        state.metric = metric;
        state.largest_sort = LargestSort {
            grouping: LargestGrouping::FilesFirst,
            column,
            ascending: false,
        };
        state.sort_largest_rows();
        assert_eq!(names(&state)[0], "small.bin");
        state.largest_sort.column = if column == LargestColumn::Allocated {
            LargestColumn::Logical
        } else {
            LargestColumn::Allocated
        };
        state.sort_largest_rows();
        assert_eq!(names(&state)[0], "root.bin");
    }
}

#[test]
fn equal_values_sort_deterministically_and_empty_rows_are_safe() {
    let mut state = state();
    state.largest_sort.column = LargestColumn::Difference;
    state.sort_largest_rows();
    let expected = state.largest_rows.clone();
    Arc::make_mut(&mut state.largest_rows).reverse();
    state.sort_largest_rows();
    assert_eq!(state.largest_rows, expected);
    state.largest_rows = Arc::new(Vec::new());
    state.sort_largest_rows();
    assert!(state.largest_rows.is_empty());
}
