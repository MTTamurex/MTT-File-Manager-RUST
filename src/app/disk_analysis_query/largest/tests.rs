use super::*;
use crate::app::disk_analysis_query::{compute_filtered_weights, DiskAnalysisFilter};
use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};
use std::sync::Arc;

fn record(
    frn: u64,
    parent: u64,
    name: &str,
    size: u64,
    allocated: u64,
    dir: bool,
) -> DiskAnalysisRecord {
    DiskAnalysisRecord {
        frn,
        parent_frn: parent,
        name: name.into(),
        size,
        allocated_size: allocated,
        is_dir: dir,
        is_reparse: false,
    }
}

fn model() -> DiskAnalysisModel {
    let mut records = vec![
        record(5, 5, "", 0, 0, true),
        record(10, 5, "folder", 0, 0, true),
        record(11, 10, "large.bin", 80_000_000_000, 0, false),
        record(
            12,
            5,
            "allocated.bin",
            30_000_000_000,
            30_000_000_000,
            false,
        ),
        record(13, 5, "empty.bin", 0, 0, false),
        record(
            14,
            10,
            "excluded.txt",
            90_000_000_000,
            90_000_000_000,
            false,
        ),
    ];
    for i in 0..150 {
        records.push(record(100 + i, 5, &format!("folder-{i}"), 0, 0, true));
        records.push(record(
            1000 + i,
            100 + i,
            &format!("file-{i}.bin"),
            40_000_000_000,
            40_000_000_000,
            false,
        ));
    }
    DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'C',
        records,
    })
}

fn sort(grouping: LargestGrouping, column: LargestColumn, ascending: bool) -> LargestSort {
    LargestSort {
        grouping,
        column,
        ascending,
    }
}

#[test]
fn logical_file_ranking_finds_nested_large_file_even_with_zero_disk_usage() {
    let model = model();
    let filter = DiskAnalysisFilter {
        extensions: vec!["bin".into()],
        ..Default::default()
    };
    let weights = compute_filtered_weights(&model, model.root, &filter, SizeMetric::Allocated);
    let filtered = ActiveWeights {
        id: 1,
        root: model.root,
        metric: SizeMetric::Allocated,
        weights: Arc::new(weights.weights),
        matches: Arc::new(weights.matches),
    };
    let rows = largest_items(
        &model,
        model.root,
        sort(LargestGrouping::FilesFirst, LargestColumn::Logical, false),
        SizeMetric::Allocated,
        Some(&filtered),
        100,
        || false,
    );
    assert_eq!(model.nodes[rows[0] as usize].name, "large.bin");
    assert_eq!(
        rows.iter()
            .filter(|&&i| !model.nodes[i as usize].is_dir)
            .count(),
        100
    );
    assert_eq!(
        rows.iter()
            .filter(|&&i| model.nodes[i as usize].is_dir)
            .count(),
        100
    );
    assert!(!rows
        .iter()
        .any(|&i| model.nodes[i as usize].name == "excluded.txt"));
}

#[test]
fn table_size_ranking_is_independent_of_the_chart_metric() {
    let model = model();
    for metric in [
        SizeMetric::Allocated,
        SizeMetric::Logical,
        SizeMetric::FileCount,
    ] {
        let rows = largest_items(
            &model,
            model.root,
            sort(LargestGrouping::FilesFirst, LargestColumn::Logical, false),
            metric,
            None,
            2,
            || false,
        );
        assert_eq!(model.nodes[rows[0] as usize].name, "excluded.txt");
        assert_eq!(model.nodes[rows[1] as usize].name, "large.bin");
    }
}

#[test]
fn ascending_selects_smallest_from_entire_tree_not_smallest_of_largest_sample() {
    let model = model();
    let rows = largest_items(
        &model,
        model.root,
        sort(LargestGrouping::FilesFirst, LargestColumn::Logical, true),
        SizeMetric::Allocated,
        None,
        1,
        || false,
    );
    assert_eq!(model.nodes[rows[0] as usize].name, "empty.bin");
}

#[test]
fn bounded_selection_matches_full_reference_sort_for_every_control_combination() {
    let model = model();
    for grouping in [
        LargestGrouping::Mixed,
        LargestGrouping::FilesFirst,
        LargestGrouping::FoldersFirst,
    ] {
        for column in LargestColumn::SORTABLE {
            for ascending in [false, true] {
                let sort = sort(grouping, column, ascending);
                // Only reachable descendants are eligible; the model also
                // contains a detached filesystem-root record.
                let mut reference = Vec::new();
                let mut pending = model.children(model.root).to_vec();
                while let Some(idx) = pending.pop() {
                    reference.push(idx);
                    pending.extend_from_slice(model.children(idx));
                }
                reference.sort_by(|&a, &b| {
                    compare_largest_rows(&model, a, b, sort, SizeMetric::Allocated, None)
                });
                if grouping != LargestGrouping::Mixed || column == LargestColumn::Files {
                    let mut counts = [0, 0];
                    reference.retain(|&idx| {
                        let slot = usize::from(model.nodes[idx as usize].is_dir);
                        counts[slot] += 1;
                        counts[slot] <= 3
                    });
                } else {
                    reference.truncate(3);
                }
                let actual = largest_items(
                    &model,
                    model.root,
                    sort,
                    SizeMetric::Allocated,
                    None,
                    3,
                    || false,
                );
                assert_eq!(actual, reference, "{sort:?}");
            }
        }
    }
}

#[test]
fn subtree_limits_cancellation_and_zero_limit_are_respected() {
    let model = model();
    let folder = model.nodes.iter().position(|n| n.name == "folder").unwrap() as u32;
    let rows = largest_items(
        &model,
        folder,
        LargestSort::default(),
        SizeMetric::Allocated,
        None,
        10,
        || false,
    );
    assert_eq!(rows.len(), 2);
    assert!(!rows.contains(&folder));
    assert!(largest_items(
        &model,
        model.root,
        LargestSort::default(),
        SizeMetric::Allocated,
        None,
        0,
        || false
    )
    .is_empty());
    assert!(largest_items(
        &model,
        model.root,
        LargestSort::default(),
        SizeMetric::Allocated,
        None,
        10,
        || true
    )
    .is_empty());
}
