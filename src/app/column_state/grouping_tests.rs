use super::*;

fn state() -> (ColumnGrouping, Sender<(u64, ColumnGroups)>) {
    let (results, receiver) = bounded(4);
    let (sender, _requests) = bounded(2);
    (
        ColumnGrouping {
            sender: Some(sender),
            receiver,
            jobs: LruCache::new(std::num::NonZeroUsize::new(4).unwrap()),
            current: [None, None],
        },
        results,
    )
}

fn job(result: Option<ColumnGroups>) -> GroupJob {
    let mut job = GroupJob::new(Arc::new(AtomicBool::new(false)));
    job.in_flight = result.is_none();
    job.latest = result.map(Arc::new);
    job
}

#[test]
fn ready_groups_wait_for_interactions_and_are_published_independently_per_pane() {
    let (mut state, _results) = state();
    state.jobs.put(1, job(Some(ColumnGroups::default())));
    let projection = Arc::new(GroupProjection::default());
    assert!(!state.publish_ready(1, 0, true, &projection));
    assert!(state.publish_ready(1, 0, false, &projection));
    state.remember_projection(1, 0, &projection);
    assert!(!state.publish_ready(1, 0, false, &projection));
    assert!(state.jobs.peek(&1).unwrap().published[1].is_none());
    assert!(state.publish_ready(1, 1, false, &projection));
}

#[test]
fn changing_one_pane_only_cancels_groups_no_longer_used_by_either_pane() {
    let (mut state, _results) = state();
    state.set_current(0, Some(1));
    state.set_current(1, Some(1));
    let pending = job(None);
    let cancelled = pending.cancelled.clone();
    state.jobs.put(1, pending);
    state.set_current(0, Some(2));
    assert!(!cancelled.load(Ordering::Acquire));
    state.set_current(1, None);
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
fn cancelled_results_are_ignored_and_deferred_reads_schedule_a_retry() {
    let (mut state, results) = state();
    state.jobs.put(2, job(None));
    results.send((1, ColumnGroups::default())).unwrap();
    results
        .send((
            2,
            ColumnGroups {
                deferred: true,
                ..Default::default()
            },
        ))
        .unwrap();
    state.poll();
    assert!(!state.jobs.contains(&1));
    assert!(state.jobs.peek(&2).unwrap().latest.is_some());
    assert!(state.jobs.peek(&2).unwrap().retry_at.is_some());
}

#[test]
fn disconnected_group_worker_releases_jobs_instead_of_leaving_them_pending() {
    let (mut state, results) = state();
    state.set_current(0, Some(1));
    state.jobs.put(1, job(None));
    drop(results);
    state.poll();
    assert!(state.sender.is_none());
    assert!(state.jobs.is_empty());
}

#[test]
fn restoring_another_tab_projection_reapplies_cached_groups_once() {
    let (mut state, _results) = state();
    state.jobs.put(1, job(Some(ColumnGroups::default())));
    let original = Arc::new(GroupProjection::default());
    assert!(state.publish_ready(1, 0, false, &original));
    state.remember_projection(1, 0, &original);
    assert!(!state.publish_ready(1, 0, false, &original));
    let restored = Arc::new(GroupProjection::default());
    assert!(state.publish_ready(1, 0, false, &restored));
    state.remember_projection(1, 0, &restored);
    assert!(!state.publish_ready(1, 0, false, &restored));
}

#[test]
fn retrying_deferred_properties_preserves_the_published_groups_until_replacement_is_ready() {
    let (mut state, results) = state();
    let mut ready = job(Some(ColumnGroups::default()));
    ready.retry_at = Some(Instant::now() - std::time::Duration::from_secs(1));
    state.jobs.put(1, ready);
    let projection = Arc::new(GroupProjection::default());
    assert!(state.publish_ready(1, 0, false, &projection));
    state.remember_projection(1, 0, &projection);
    let published = state.jobs.peek(&1).unwrap().published[0].clone().unwrap();
    let ctx = egui::Context::default();
    assert!(state.needs_request(1, Instant::now(), &ctx));
    state.record_request(1, Arc::new(AtomicBool::new(false)));
    assert!(!state.needs_request(1, Instant::now(), &ctx));
    assert!(Arc::ptr_eq(
        &published,
        state.jobs.peek(&1).unwrap().published[0].as_ref().unwrap()
    ));
    assert!(!state.publish_ready(1, 0, false, &projection));
    results.send((1, ColumnGroups::default())).unwrap();
    state.poll();
    assert!(!state.publish_ready(1, 0, true, &projection));
    assert!(Arc::ptr_eq(
        &published,
        state.jobs.peek(&1).unwrap().published[0].as_ref().unwrap()
    ));
    assert!(state.publish_ready(1, 1, false, &projection));
    assert!(Arc::ptr_eq(
        &published,
        state.jobs.peek(&1).unwrap().published[0].as_ref().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &published,
        state.jobs.peek(&1).unwrap().published[1].as_ref().unwrap()
    ));
    assert!(state.publish_ready(1, 0, false, &projection));
    assert!(!Arc::ptr_eq(
        &published,
        state.jobs.peek(&1).unwrap().published[0].as_ref().unwrap()
    ));
}
