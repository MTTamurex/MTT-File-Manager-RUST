use super::super::sorting_worker::Value;
use super::*;
use crate::domain::file_entry::FoldersPosition;

#[test]
fn missing_values_stay_last_in_both_directions() {
    for descending in [true, false] {
        assert!(Value::Missing
            .compare(&Value::Text("value".into()), descending)
            .is_gt());
    }
}

#[test]
fn text_sort_uses_natural_order() {
    assert!(Value::Text("item2".into())
        .compare(&Value::Text("item10".into()), false)
        .is_lt());
}

#[test]
fn applying_a_sort_does_not_change_the_listing_fingerprint() {
    let mut sorting = ColumnSorting::new();
    let a = FileEntry::from_path("file1.txt".into(), false);
    let b = FileEntry::from_path("file2.txt".into(), false);
    assert_eq!(
        sorting.fingerprint(&Arc::new(vec![a.clone(), b.clone()])),
        sorting.fingerprint(&Arc::new(vec![b, a]))
    );
}

#[test]
fn cancelling_a_job_prevents_its_result_from_being_published() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let job = SortJob {
        cancelled: cancelled.clone(),
        ranks: None,
        applied: [std::sync::Weak::new(), std::sync::Weak::new()],
        retry_at: None,
    };
    drop(job);
    assert!(cancelled.load(Ordering::Relaxed));
}

#[test]
fn tags_sort_the_complete_listing_and_keep_missing_values_last() {
    use crate::domain::file_tag::{normalize_tag_path_key, FileTag, TagColor};
    let items = Arc::new(vec![
        FileEntry::from_path("file1.txt".into(), false),
        FileEntry::from_path("file2.txt".into(), false),
        FileEntry::from_path("file3.txt".into(), false),
    ]);
    let definitions = rustc_hash::FxHashMap::from_iter([
        (
            1,
            FileTag {
                id: 1,
                name: "Beta".into(),
                color: TagColor::Red,
                position: 0,
            },
        ),
        (
            2,
            FileTag {
                id: 2,
                name: "Alpha".into(),
                color: TagColor::Red,
                position: 1,
            },
        ),
    ]);
    let tags = Arc::new(rustc_hash::FxHashMap::from_iter([
        (normalize_tag_path_key(&items[0].path), vec![1]),
        (normalize_tag_path_key(&items[1].path), vec![2]),
    ]));
    for descending in [false, true] {
        let request = SortRequest {
            signature: 1,
            items: items.clone(),
            column: ColumnId::Tags,
            descending,
            folders: FoldersPosition::Mixed,
            tags: tags.clone(),
            definitions: definitions.clone(),
            cancelled: Arc::new(AtomicBool::new(false)),
            ctx: egui::Context::default(),
        };
        let ranks = run(&request).unwrap().ranks;
        assert_eq!(ranks.len(), 3);
        assert_eq!(ranks[&items[2].path], 2);
        assert_eq!(ranks[&items[usize::from(!descending)].path], 0);
        request.cancelled.store(true, Ordering::Relaxed);
        assert!(run(&request).is_none());
    }
}

fn pending_job() -> SortJob {
    SortJob {
        cancelled: Arc::new(AtomicBool::new(false)),
        ranks: None,
        applied: [std::sync::Weak::new(), std::sync::Weak::new()],
        retry_at: None,
    }
}

#[test]
fn ready_orders_wait_until_index_based_interactions_finish() {
    let mut sorting = ColumnSorting::new();
    let mut items = Arc::new(vec![
        FileEntry::from_path("file1.txt".into(), false),
        FileEntry::from_path("file2.txt".into(), false),
    ]);
    let mut job = pending_job();
    job.ranks = Some(HashMap::from([
        (items[0].path.clone(), 1),
        (items[1].path.clone(), 0),
    ]));
    sorting.jobs.put(1, job);
    let before = items.clone();
    assert!(!sorting.apply_ready(1, 0, &mut items, true));
    assert!(Arc::ptr_eq(&items, &before));
    assert!(sorting.apply_ready(1, 0, &mut items, false));
    assert_eq!(items[0].path, before[1].path);
    assert!(!sorting.apply_ready(1, 0, &mut items, false));
}

#[test]
fn changing_one_pane_does_not_cancel_a_job_still_used_by_the_other() {
    let mut sorting = ColumnSorting::new();
    let job = pending_job();
    let cancelled = job.cancelled.clone();
    sorting.jobs.put(1, job);
    sorting.set_current(0, Some(1));
    sorting.set_current(1, Some(1));
    sorting.set_current(0, None);
    assert!(!cancelled.load(Ordering::Relaxed));
    sorting.set_current(1, None);
    assert!(cancelled.load(Ordering::Relaxed));
}

#[test]
fn a_disconnected_sort_worker_cannot_leave_a_job_permanently_pending() {
    let mut sorting = ColumnSorting::new();
    let (results, receiver) = bounded(1);
    sorting.receiver = receiver;
    let job = pending_job();
    let cancelled = job.cancelled.clone();
    sorting.jobs.put(1, job);
    sorting.set_current(0, Some(1));
    drop(results);
    sorting.poll();
    assert!(!sorting.pending(0));
    assert!(sorting.sender.is_none());
    assert!(cancelled.load(Ordering::Relaxed));
}

#[test]
fn fingerprints_track_name_directory_kind_and_original_location() {
    use crate::domain::file_entry::RecycleBinMeta;
    let mut sorting = ColumnSorting::new();
    let item = FileEntry::from_path("file.txt".into(), false);
    let original = sorting.fingerprint(&Arc::new(vec![item.clone()]));
    let mut renamed = item.clone();
    renamed.name = "renamed.txt".into();
    assert_ne!(original, sorting.fingerprint(&Arc::new(vec![renamed])));
    let mut directory = item.clone();
    directory.is_dir = true;
    assert_ne!(original, sorting.fingerprint(&Arc::new(vec![directory])));
    let mut recycled = item;
    recycled.recycle_bin = Some(Box::new(RecycleBinMeta {
        deletion_date: String::new(),
        original_path: "folder/file.txt".into(),
    }));
    assert_ne!(original, sorting.fingerprint(&Arc::new(vec![recycled])));
}

#[test]
fn temporarily_unreadable_properties_schedule_a_later_sort_retry() {
    let request = SortRequest {
        signature: 1,
        items: Arc::new(vec![FileEntry::from_path("file.txt.part".into(), false)]),
        column: ColumnId::Property(crate::domain::list_columns::PropertyId { fmtid: 0, pid: 1 }),
        descending: false,
        folders: FoldersPosition::Mixed,
        tags: Arc::default(),
        definitions: Default::default(),
        cancelled: Arc::new(AtomicBool::new(false)),
        ctx: egui::Context::default(),
    };
    let result = run(&request).unwrap();
    assert!(result.deferred);
    let mut sorting = ColumnSorting::new();
    sorting.jobs.put(1, pending_job());
    let (sender, receiver) = bounded(1);
    sorting.receiver = receiver;
    sender.send((1, result)).unwrap();
    sorting.poll();
    assert!(sorting.jobs.peek(&1).unwrap().retry_at.unwrap() > std::time::Instant::now());
}
