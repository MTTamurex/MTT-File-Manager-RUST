//! Permanently remove only explicitly selected Recycle Bin namespace items.
use super::{
    identity, live_progress::LiveProgress, reconciliation, sink, tracking, RefreshOnDrop, Running,
};
use crate::infrastructure::copy_engine::{CopyRuntime, Outcome, OutcomeKind};
use crate::infrastructure::delete_engine::confirmation;
use crate::infrastructure::windows::recycle_bin::{self, RecycleBinShellItem};
use crate::workers::file_operation_worker::{FileOperationRequest, FileOperationResult, SendHwnd};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{atomic::Ordering, mpsc::Sender, Arc};

pub(super) fn try_handle(
    request: &FileOperationRequest,
    sender: &Sender<FileOperationResult>,
    runtime: &Arc<CopyRuntime>,
) -> bool {
    let (physical_paths, hwnd) = match request {
        FileOperationRequest::DeletePermanently {
            physical_paths,
            hwnd,
        } => (physical_paths, hwnd),
        // Delete inside the Recycle Bin is permanent too. Keyboard Delete and
        // the ordinary delete action use this request, unlike the bin toolbar.
        FileOperationRequest::Delete { paths, hwnd } => (paths, hwnd),
        _ => return false,
    };
    if !runtime.enabled.load(Ordering::Relaxed) || !is_recycle_bin_request(physical_paths) {
        return false;
    }
    let Some(_running) = Running::acquire(runtime) else {
        return true;
    };
    let _refresh = RefreshOnDrop(sender);
    handle(physical_paths, *hwnd, runtime);
    true
}

pub(super) fn is_recycle_bin_request(paths: &[PathBuf]) -> bool {
    !paths.is_empty() && paths.iter().all(|path| normalized_path(path).is_some())
}

fn handle(paths: &[PathBuf], hwnd: SendHwnd, runtime: &Arc<CopyRuntime>) {
    let label = rust_i18n::t!("delete_engine.recycle_bin").to_string();
    let job = runtime.start_delete_with_label(Vec::new(), true, Some(label));
    let operation = &job.operation;
    let progress = operation.deletion.as_ref().unwrap();

    let selected = match select_items(paths, || operation.progress.is_cancelled()) {
        Ok(items) => items,
        Err(error) => {
            let cancelled =
                operation.progress.is_cancelled() || tracking::is_cancellation(error.code());
            job.finish(Outcome {
                kind: if cancelled {
                    OutcomeKind::Cancelled
                } else {
                    OutcomeKind::Failed
                },
                message: (!cancelled).then(|| tracking::error_message(&error)),
            });
            return;
        }
    };
    let total = selected.len();
    progress.total.store(total as u64, Ordering::Relaxed);
    if total == 0 {
        job.finish(Outcome {
            kind: OutcomeKind::Failed,
            message: Some(rust_i18n::t!("operations.selection_operation_blocked").to_string()),
        });
        return;
    }
    if !confirmation::ask_selected_recycle_bin_items(operation, total)
        || operation.progress.is_cancelled()
    {
        job.finish(Outcome {
            kind: OutcomeKind::Cancelled,
            message: None,
        });
        return;
    }

    let snapshot = reconciliation::Snapshot::capture(&selected);
    let tracking = Arc::new(tracking::Tracking::new(operation.clone(), total));
    let sink = sink::global(tracking.clone());
    let observer = LiveProgress::start(snapshot.clone(), tracking.clone()).ok();
    let result = recycle_bin::delete_recycle_bin_items_with_progress(
        hwnd.0,
        &selected,
        &sink,
        |index, item| sink::item(tracking.clone(), index, item),
        || operation.progress.is_cancelled(),
    );
    drop(observer);
    snapshot.reconcile(&tracking);
    job.finish(tracking.outcome(result));
}

fn select_items(
    paths: &[PathBuf],
    cancelled: impl Fn() -> bool,
) -> windows::core::Result<Vec<RecycleBinShellItem>> {
    if cancelled() {
        return Err(windows::Win32::Foundation::E_ABORT.into());
    }
    let items = recycle_bin::enumerate_recycle_bin_shell_items(&cancelled)?;
    select_from_snapshot(
        paths,
        items,
        |item| identity::parsing_name(&item.item),
        cancelled,
    )
}

fn select_from_snapshot<T>(
    paths: &[PathBuf],
    items: Vec<T>,
    parsing_name: impl Fn(&T) -> Option<String>,
    cancelled: impl Fn() -> bool,
) -> windows::core::Result<Vec<T>> {
    let Some(requested): Option<HashSet<_>> =
        paths.iter().map(|path| normalized_path(path)).collect()
    else {
        return Ok(Vec::new());
    };
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let mut matched = HashSet::with_capacity(requested.len());
    let mut selected = Vec::with_capacity(requested.len());
    for item in items {
        if cancelled() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_ABORT,
            ));
        }
        let Some(name) = parsing_name(&item) else {
            continue;
        };
        if let Some(name) =
            normalized_path(Path::new(&name)).filter(|name| requested.contains(name))
        {
            // A duplicate identity cannot stand in for another missing selection.
            // Ambiguous snapshots must never authorize a partial/wider deletion.
            if !matched.insert(name) {
                return Ok(Vec::new());
            }
            selected.push(item);
        }
    }
    if selected.len() != requested.len() {
        return Ok(Vec::new());
    }
    Ok(selected)
}

fn normalized_path(path: &Path) -> Option<String> {
    let text = path
        .to_str()?
        .strip_prefix(r"\\?\")
        .unwrap_or(path.to_str()?);
    const BIN_NAMESPACE: &str = "::{645ff040-5081-101b-9f08-00aa002f954e}";
    let lower = text.to_lowercase();
    if let Some(child) = lower
        .strip_prefix(BIN_NAMESPACE)
        .and_then(|rest| rest.strip_prefix('\\'))
    {
        // Accept children, never the namespace root itself or another CLSID.
        if !child.is_empty()
            && child
                .split('\\')
                .all(|part| !part.is_empty() && part != "." && part != "..")
        {
            return Some(lower);
        }
        return None;
    }
    let components: Vec<_> = path.components().collect();
    let is_recycle_bin_entry = components.len() >= 4
        && matches!(components[0], Component::Prefix(_))
        && matches!(components[1], Component::RootDir)
        && matches!(components[2], Component::Normal(name)
            if name.to_string_lossy().eq_ignore_ascii_case("$Recycle.Bin"))
        && components[3..]
            .iter()
            .all(|component| matches!(component, Component::Normal(_)));
    if !path.is_absolute() || !is_recycle_bin_entry {
        return None;
    }
    Some(
        path.to_str()?
            .trim_start_matches(r"\\?\")
            .replace('/', r"\")
            .to_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_recycle_bin_request_requires_absolute_physical_paths() {
        assert!(is_recycle_bin_request(&[PathBuf::from(
            r"C:\$Recycle.Bin\entry"
        )]));
        assert!(!is_recycle_bin_request(&[PathBuf::from(
            r"relative\$Recycle.Bin\entry"
        )]));
        assert!(!is_recycle_bin_request(&[PathBuf::from(
            r"C:\ordinary\entry"
        )]));
        assert!(!is_recycle_bin_request(&[PathBuf::from(
            r"C:\ordinary\$Recycle.Bin\entry"
        )]));
        assert!(!is_recycle_bin_request(&[PathBuf::from(
            r"C:\$Recycle.Bin"
        )]));
        assert!(!is_recycle_bin_request(&[]));
    }

    #[test]
    fn selected_paths_are_matched_case_insensitively_and_normalize_verbatim_prefix() {
        assert_eq!(
            normalized_path(Path::new(r"\\?\C:\$Recycle.Bin\ABC")),
            normalized_path(Path::new(r"c:\$recycle.bin\abc")),
        );
    }

    fn filter(paths: &[&str], snapshot: &[&str]) -> Vec<String> {
        let paths: Vec<_> = paths.iter().map(PathBuf::from).collect();
        select_from_snapshot(
            &paths,
            snapshot.iter().map(|name| name.to_string()).collect(),
            |name| Some(name.clone()),
            || false,
        )
        .unwrap()
    }

    #[test]
    fn snapshot_filter_selects_only_exact_roots_not_children_or_similar_names() {
        let root = r"C:\$Recycle.Bin\entry";
        assert_eq!(
            filter(
                &[root],
                &[
                    r"C:\$Recycle.Bin\other",
                    r"C:\$Recycle.Bin\entry\child.txt",
                    r"C:\$Recycle.Bin\entry-other",
                    root,
                ],
            ),
            vec![root],
        );
    }

    #[test]
    fn snapshot_filter_rejects_missing_and_duplicate_identities() {
        let first = r"C:\$Recycle.Bin\entry";
        let second = r"C:\$Recycle.Bin\other";
        assert!(filter(&[first, second], &[first]).is_empty());
        assert!(filter(&[first, second], &[first, first]).is_empty());
        assert!(filter(&[first], &[first, first]).is_empty());
        assert_eq!(
            filter(&[first, second], &[second, first]),
            vec![second, first]
        );
    }

    #[test]
    fn snapshot_filter_deduplicates_requested_paths_without_expanding_scope() {
        let root = r"C:\$Recycle.Bin\entry";
        assert_eq!(filter(&[root, root], &[root]), vec![root]);
        assert!(filter(&[root, r"C:\ordinary\file.txt"], &[root]).is_empty());
    }

    #[test]
    fn snapshot_filter_observes_cancellation_without_returning_partial_selection() {
        let path = PathBuf::from(r"C:\$Recycle.Bin\entry");
        let snapshot = vec![path.clone()];
        let result = select_from_snapshot(
            std::slice::from_ref(&path),
            snapshot,
            |path| path.to_str().map(str::to_owned),
            || true,
        );
        assert!(result.is_err_and(|error| error.code() == windows::Win32::Foundation::E_ABORT));
    }

    #[test]
    fn malformed_utf16_paths_are_not_lossily_matched_to_another_identity() {
        use std::os::windows::ffi::OsStringExt;
        let mut units: Vec<_> = r"C:\$Recycle.Bin\entry".encode_utf16().collect();
        units.push(0xD800);
        let path = PathBuf::from(std::ffi::OsString::from_wide(&units));
        assert!(normalized_path(&path).is_none());
    }

    #[test]
    fn namespace_snapshot_filter_matches_selected_children_only() {
        let selected = r"::{645FF040-5081-101B-9F08-00AA002F954E}\entry";
        let other = r"::{645FF040-5081-101B-9F08-00AA002F954E}\other";
        assert_eq!(
            filter(
                &[r"\\?\::{645ff040-5081-101b-9f08-00aa002f954e}\entry"],
                &[other, selected],
            ),
            vec![selected],
        );
        for invalid in [
            r"::{645FF040-5081-101B-9F08-00AA002F954E}",
            r"::{645FF040-5081-101B-9F08-00AA002F954E}\",
            r"::{645FF040-5081-101B-9F08-00AA002F954E}\..\entry",
            r"::{00000000-0000-0000-0000-000000000000}\entry",
        ] {
            assert!(!is_recycle_bin_request(&[invalid.into()]));
        }
    }
}
