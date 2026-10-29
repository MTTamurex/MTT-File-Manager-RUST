use super::*;
use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

fn state_with_queries() -> DiskAnalysisState {
    let mut state = DiskAnalysisState::new();
    state.model = Some(Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'C',
        records: vec![DiskAnalysisRecord {
            frn: 5,
            parent_frn: 5,
            name: String::new(),
            size: 0,
            allocated_size: 0,
            is_dir: true,
            is_reparse: false,
        }],
    })));
    state.search_text = "file".to_string();
    state.sync_query_jobs();
    state
}

#[test]
fn unchanged_filter_reschedules_cleared_queries() {
    let mut state = state_with_queries();
    let old_jobs = (state.largest_job, state.search_job);

    // Both Clear with default inputs and All with every category selected
    // notify the state even though the effective filter is unchanged.
    state.mark_filter_changed();
    state.sync_query_jobs();

    assert!(!state.filter.is_active());
    assert!(state.largest_job > old_jobs.0);
    assert!(state.search_job > old_jobs.1);
    assert!(state.largest_pending);
    assert!(state.search_worker_pending);
}

#[test]
fn clearing_filter_before_weights_finish_reschedules_unfiltered_queries() {
    let mut state = state_with_queries();
    state.filter_categories_mask = 1;
    state.mark_filter_changed();
    state.filter_pending_at = Some(Instant::now() - QUERY_DEBOUNCE);
    state.sync_query_jobs();
    assert!(state.weights_pending);
    let old_filter_job = state.filter_job;

    state.filter_categories_mask = DiskAnalysisFilter::all_categories_mask();
    state.mark_filter_changed();
    state.sync_query_jobs();

    assert!(!state.filter.is_active());
    assert!(!state.weights_pending);
    assert!(state.largest_pending);
    assert!(state.search_worker_pending);
    assert!(!state.apply_query_message(QueryMessage::Weights {
        job: old_filter_job,
        root: state.current_root().unwrap(),
        metric: state.metric,
        weights: Arc::new(vec![0]),
        matches: Arc::new(vec![false]),
    }));
    assert!(state.active_weights.is_none());
}

#[test]
fn column_direction_and_grouping_changes_reselect_rows_and_reject_stale_results() {
    use crate::app::disk_analysis_sorting::LargestGrouping;
    let mut state = state_with_queries();
    for step in 0..3 {
        let old_job = state.largest_job;
        let old_search = state.search_job;
        match step {
            0 => state.largest_sort.column = LargestColumn::Logical,
            1 => state.largest_sort.ascending = true,
            _ => state.largest_sort.grouping = LargestGrouping::FilesFirst,
        }
        state.sync_query_jobs();
        assert!(state.largest_job > old_job);
        assert_eq!(
            state.largest_generation.load(Ordering::Acquire),
            state.largest_job
        );
        assert!(state.largest_is_updating());
        assert_eq!(state.search_job, old_search);
        assert!(!state.apply_query_message(QueryMessage::Largest {
            job: old_job,
            rows: Arc::new(vec![99])
        }));
        assert!(state.largest_rows.is_empty());
        let job = state.largest_job;
        state.sync_query_jobs();
        assert_eq!(state.largest_job, job);
    }
}

#[test]
fn query_worker_uses_requested_table_order_not_chart_metric() {
    use crate::app::disk_analysis_sorting::LargestGrouping;
    let model = Arc::new(DiskAnalysisModel::build(DiskAnalysisSnapshot {
        drive_letter: 'C',
        records: vec![
            DiskAnalysisRecord {
                frn: 5,
                parent_frn: 5,
                name: String::new(),
                size: 0,
                allocated_size: 0,
                is_dir: true,
                is_reparse: false,
            },
            DiskAnalysisRecord {
                frn: 6,
                parent_frn: 5,
                name: "large.bin".into(),
                size: 80_000_000_000,
                allocated_size: 0,
                is_dir: false,
                is_reparse: false,
            },
            DiskAnalysisRecord {
                frn: 7,
                parent_frn: 5,
                name: "allocated.bin".into(),
                size: 30_000_000_000,
                allocated_size: 30_000_000_000,
                is_dir: false,
                is_reparse: false,
            },
        ],
    }));
    let (requests, receiver) = channel();
    let (results, responses) = channel();
    requests
        .send(QueryRequest::Largest {
            job: 1,
            cancel: Arc::new(AtomicU64::new(1)),
            model: model.clone(),
            root: model.root,
            metric: SizeMetric::Allocated,
            sort: LargestSort {
                grouping: LargestGrouping::FilesFirst,
                column: LargestColumn::Logical,
                ascending: false,
            },
            filtered: None,
        })
        .unwrap();
    drop(requests);
    query_worker_loop(receiver, results);
    let QueryMessage::Largest { job, rows } = responses.recv().unwrap() else {
        panic!("expected ranked rows")
    };
    assert_eq!(job, 1);
    assert_eq!(model.nodes[rows[0] as usize].name, "large.bin");
}
