use super::*;

fn state() -> (ColumnGrouping, Sender<(u64, ColumnGroups)>) {
    let (results, receiver) = bounded(4);
    let (sender, _requests) = bounded(2);
    (
        ColumnGrouping {
            sender: Some(sender),
            receiver,
            jobs: HashMap::new(),
            current: HashMap::new(),
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

fn published(state: &ColumnGrouping, pane: usize) -> &Arc<ColumnGroups> {
    &state.jobs[&1].published[&GroupOwner::Main(pane)].groups
}

#[test]
fn ready_groups_wait_for_interactions_and_are_published_independently_per_pane() {
    let (mut state, _results) = state();
    state.jobs.insert(1, job(Some(ColumnGroups::default())));
    let projection = Arc::new(GroupProjection::default());
    assert!(!state.publish_ready(1, 0, true, &projection));
    assert!(state.publish_ready(1, 0, false, &projection));
    state.remember_projection(1, 0, &projection);
    assert!(!state.publish_ready(1, 0, false, &projection));
    assert!(!state.jobs[&1].published.contains_key(&GroupOwner::Main(1)));
    assert!(state.publish_ready(1, 1, false, &projection));
}

#[test]
fn changing_one_pane_only_cancels_groups_no_longer_used_by_either_pane() {
    let (mut state, _results) = state();
    state.set_current(0, Some(1));
    state.set_current(1, Some(1));
    let pending = job(None);
    let cancelled = pending.cancelled.clone();
    state.jobs.insert(1, pending);
    state.set_current(0, Some(2));
    assert!(!cancelled.load(Ordering::Acquire));
    state.set_current(1, None);
    assert!(cancelled.load(Ordering::Acquire));
}

#[test]
fn cancelled_results_are_ignored_and_deferred_reads_schedule_a_retry() {
    let (mut state, results) = state();
    state.jobs.insert(2, job(None));
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
    assert!(!state.jobs.contains_key(&1));
    assert!(state.jobs[&2].latest.is_some());
    assert!(state.jobs[&2].retry_at.is_some());
}

#[test]
fn disconnected_group_worker_releases_jobs_instead_of_leaving_them_pending() {
    let (mut state, results) = state();
    state.set_current(0, Some(1));
    state.jobs.insert(1, job(None));
    drop(results);
    state.poll();
    assert!(state.sender.is_none());
    assert!(state.jobs.is_empty());
}

#[test]
fn restoring_another_tab_projection_reapplies_cached_groups_once() {
    let (mut state, _results) = state();
    state.jobs.insert(1, job(Some(ColumnGroups::default())));
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
    state.jobs.insert(1, ready);
    let projection = Arc::new(GroupProjection::default());
    assert!(state.publish_ready(1, 0, false, &projection));
    state.remember_projection(1, 0, &projection);
    let previous = published(&state, 0).clone();
    let ctx = egui::Context::default();
    assert!(state.needs_request(1, Instant::now(), &ctx));
    state.record_request(1, Arc::new(AtomicBool::new(false)));
    assert!(!state.needs_request(1, Instant::now(), &ctx));
    assert!(Arc::ptr_eq(&previous, published(&state, 0)));
    assert!(!state.publish_ready(1, 0, false, &projection));
    results.send((1, ColumnGroups::default())).unwrap();
    state.poll();
    assert!(!state.publish_ready(1, 0, true, &projection));
    assert!(Arc::ptr_eq(&previous, published(&state, 0)));
    assert!(state.publish_ready(1, 1, false, &projection));
    assert!(Arc::ptr_eq(&previous, published(&state, 0)));
    assert!(!Arc::ptr_eq(&previous, published(&state, 1)));
    assert!(state.publish_ready(1, 0, false, &projection));
    assert!(!Arc::ptr_eq(&previous, published(&state, 0)));
}

#[test]
fn miller_ancestors_have_independent_ownership_without_evicting_other_active_directories() {
    let (mut state, _results) = state();
    let keep: HashSet<_> = (0..12)
        .map(|index| PathBuf::from(format!("folder_{index}")))
        .collect();
    for (index, path) in keep.iter().enumerate() {
        let signature = index as u64;
        state.set_owner(GroupOwner::Ancestor(0, path.clone()), Some(signature));
        state
            .jobs
            .insert(signature, job(Some(ColumnGroups::default())));
    }
    assert_eq!(state.jobs.len(), 12);
    let shared_path = keep.iter().next().unwrap().clone();
    let signature = state.current[&GroupOwner::Ancestor(0, shared_path.clone())];
    state.set_owner(
        GroupOwner::Ancestor(1, shared_path.clone()),
        Some(signature),
    );
    let token = state.jobs[&signature].cancelled.clone();
    state.retain_ancestors(0, &HashSet::new());
    assert_eq!(state.jobs.len(), 1);
    assert!(!token.load(Ordering::Acquire));
    state.retain_ancestors(1, &HashSet::new());
    assert!(state.jobs.is_empty());
    assert!(token.load(Ordering::Acquire));
}

#[test]
fn publishing_one_miller_directory_cannot_replace_another_directorys_projection() {
    let (mut state, _results) = state();
    let a = GroupOwner::Ancestor(0, "parent".into());
    let b = GroupOwner::Ancestor(0, "parent/child".into());
    let projection_a = Arc::new(GroupProjection::default());
    let projection_b = Arc::new(GroupProjection::default());
    state.set_owner(a.clone(), Some(1));
    state.set_owner(b.clone(), Some(2));
    state.jobs.insert(1, job(Some(ColumnGroups::default())));
    state.jobs.insert(2, job(Some(ColumnGroups::default())));
    assert!(state.publish_owner(1, &a, false, &projection_a));
    state.remember_owner(1, &a, &projection_a);
    assert!(state.publish_owner(2, &b, false, &projection_b));
    state.remember_owner(2, &b, &projection_b);
    assert!(!state.publish_owner(1, &a, false, &projection_a));
    assert!(!state.publish_owner(2, &b, false, &projection_b));
    assert!(!state.jobs[&1].published.contains_key(&b));
}
