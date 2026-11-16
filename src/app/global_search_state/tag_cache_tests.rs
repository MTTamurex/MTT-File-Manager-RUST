use super::*;

fn state() -> GlobalSearchState {
    let (sender, _) = std::sync::mpsc::channel();
    let (_, receiver) = std::sync::mpsc::channel();
    GlobalSearchState::new(sender, receiver)
}

#[test]
fn unrelated_tag_edits_do_not_refilter_or_resort_an_unfiltered_search() {
    let mut state = state();
    let assignments = rustc_hash::FxHashMap::default();
    state.ensure_sorted_indices(&assignments, 1, &GlobalSearchTagFilter::All);
    let filter_key = state.filter_cache_key.clone();
    let sort_key = state.sort_cache_key.clone();
    let generation = state.results_generation;
    state.invalidate_tag_assignment_dependent_results();
    state.ensure_sorted_indices(&assignments, 2, &GlobalSearchTagFilter::All);
    assert_eq!(state.filter_cache_key, filter_key);
    assert_eq!(state.sort_cache_key, sort_key);
    assert_eq!(state.results_generation, generation);
}

#[test]
fn tag_filtered_search_still_invalidates_on_assignment_changes() {
    let mut state = state();
    state.tag_filter = GlobalSearchTagFilter::Any;
    let assignments = rustc_hash::FxHashMap::default();
    state.ensure_sorted_indices(&assignments, 1, &GlobalSearchTagFilter::Any);
    let generation = state.results_generation;
    state.invalidate_tag_assignment_dependent_results();
    assert_ne!(state.results_generation, generation);
    state.ensure_sorted_indices(&assignments, 2, &GlobalSearchTagFilter::Any);
    assert_eq!(state.filter_cache_key.3, 2);
}
