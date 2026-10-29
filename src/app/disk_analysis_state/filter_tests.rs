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
