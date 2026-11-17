use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub(crate) struct BatchOutcome {
    /// Selected roots which the Shell explicitly reported as unrecyclable.
    pub(crate) unavailable: Vec<PathBuf>,
    /// Selected roots which were not started because the Shell stopped early.
    pub(crate) pending: Vec<PathBuf>,
    /// Selected roots which were attempted but not confirmed as removed.
    pub(crate) failed: Vec<PathBuf>,
    pub(crate) cancelled: bool,
    /// The batch cannot safely continue or retry.
    pub(crate) fatal: bool,
}

#[derive(Default)]
struct ItemTracking {
    attempted: HashSet<String>,
    unavailable: HashSet<String>,
    failed: HashSet<String>,
    unknown_unavailable: bool,
    unknown_item: bool,
}

pub(super) struct BatchTracking {
    requested: Vec<PathBuf>,
    pub(super) stopped: AtomicBool,
    pub(super) unavailable_seen: AtomicBool,
    items: Mutex<ItemTracking>,
}

impl BatchTracking {
    pub(super) fn new(requested: &[PathBuf]) -> Self {
        Self {
            requested: requested.to_vec(),
            stopped: AtomicBool::new(false),
            unavailable_seen: AtomicBool::new(false),
            items: Mutex::new(ItemTracking::default()),
        }
    }

    fn selected_root(&self, path: &Path) -> Option<PathBuf> {
        let candidate = path_key(path);
        self.requested
            .iter()
            .filter_map(|root| {
                let root_key = path_key(root);
                let within = candidate == root_key
                    || candidate
                        .strip_prefix(&root_key)
                        .is_some_and(|tail| root_key.ends_with('\\') || tail.starts_with('\\'));
                within.then_some((root_key.len(), root.clone()))
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, root)| root)
    }

    pub(super) fn attempted(&self, path: Option<&Path>) -> bool {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(root) = path.and_then(|path| self.selected_root(path)) {
            items.attempted.insert(path_key(&root));
            true
        } else {
            items.unknown_item = true;
            self.stopped.store(true, Ordering::Relaxed);
            false
        }
    }

    pub(super) fn unavailable(&self, path: Option<&Path>) {
        self.stopped.store(true, Ordering::Relaxed);
        self.unavailable_seen.store(true, Ordering::Relaxed);
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(root) = path.and_then(|path| self.selected_root(path)) {
            items.unavailable.insert(path_key(&root));
        } else if items.unavailable.is_empty() {
            items.unknown_unavailable = true;
        }
    }

    pub(super) fn failed(&self, path: Option<&Path>) {
        let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(root) = path.and_then(|path| self.selected_root(path)) {
            items.failed.insert(path_key(&root));
        } else {
            items.unknown_item = true;
            self.stopped.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn classify(
        &self,
        paths: &[PathBuf],
        confirmed: &HashSet<String>,
        cancelled: bool,
        fatal: bool,
        unavailable_error: bool,
    ) -> BatchOutcome {
        let items = self.items.lock().unwrap_or_else(|e| e.into_inner());
        let unavailable_seen = self.unavailable_seen.load(Ordering::Relaxed) || unavailable_error;
        let fatal = fatal || items.unknown_unavailable || items.unknown_item;
        let mut outcome = BatchOutcome {
            cancelled,
            fatal,
            ..Default::default()
        };

        for path in paths {
            let key = path_key(path);
            if confirmed.contains(&key) {
                continue;
            }
            if cancelled {
                outcome.pending.push(path.clone());
            } else if items.unavailable.contains(&key) {
                outcome.unavailable.push(path.clone());
            } else if fatal || items.failed.contains(&key) || items.attempted.contains(&key) {
                outcome.failed.push(path.clone());
            } else {
                outcome.pending.push(path.clone());
            }
        }

        if unavailable_seen && outcome.unavailable.is_empty() {
            outcome.fatal = true;
            outcome.pending.clear();
            for path in paths {
                if !confirmed.contains(&path_key(path))
                    && !outcome
                        .failed
                        .iter()
                        .any(|failed| path_key(failed) == path_key(path))
                {
                    outcome.failed.push(path.clone());
                }
            }
        }
        outcome
    }
}

pub(super) fn path_key(path: &Path) -> String {
    let path = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let path = if let Some(unc) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    };
    let is_drive_root = path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().get(2) == Some(&b'\\')
        && path.len() == 3;
    if is_drive_root {
        path
    } else {
        path.trim_end_matches('\\').to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_paths_map_to_the_selected_root_without_case_sensitivity() {
        let tracking = BatchTracking::new(&[PathBuf::from(r"C:\Data\Folder")]);
        tracking.attempted(Some(Path::new(r"c:\data\folder\child.txt")));
        tracking.unavailable(Some(Path::new(r"\\?\C:\DATA\FOLDER\locked.txt")));
        let items = tracking.items.lock().unwrap();
        assert!(items
            .attempted
            .contains(&path_key(Path::new(r"C:\Data\Folder"))));
        assert!(items
            .unavailable
            .contains(&path_key(Path::new(r"C:\Data\Folder"))));
        assert!(!items.unknown_unavailable);
        assert!(!items.unknown_item);
    }

    #[test]
    fn path_keys_ignore_trailing_separators_without_collapsing_drive_roots() {
        assert_eq!(
            path_key(Path::new(r"C:\Data\Folder\")),
            path_key(Path::new("c:/data/folder"))
        );
        assert_eq!(path_key(Path::new(r"C:\")), r"c:\");
        assert_eq!(
            path_key(Path::new(r"\\?\UNC\server\share\")),
            path_key(Path::new(r"\\server\share"))
        );
    }

    #[test]
    fn unavailable_item_is_separated_from_unstarted_batch_entries() {
        let paths = vec![
            PathBuf::from(r"C:\Data\a.txt"),
            PathBuf::from(r"C:\Data\b.txt"),
            PathBuf::from(r"C:\Data\c.txt"),
        ];
        let tracking = BatchTracking::new(&paths);
        tracking.attempted(Some(&paths[0]));
        tracking.unavailable(Some(&paths[1]));
        let confirmed = HashSet::from([path_key(&paths[0])]);

        let outcome = tracking.classify(&paths, &confirmed, false, false, false);
        assert_eq!(outcome.unavailable, vec![paths[1].clone()]);
        assert_eq!(outcome.pending, vec![paths[2].clone()]);
        assert!(outcome.failed.is_empty());
        assert!(!outcome.fatal);
    }

    #[test]
    fn unknown_unavailable_callback_fails_closed_without_fallback_authorization() {
        let paths = vec![PathBuf::from(r"C:\Data\a.txt")];
        let tracking = BatchTracking::new(&paths);
        tracking.unavailable(None);

        let outcome = tracking.classify(&paths, &HashSet::new(), false, false, false);
        assert!(outcome.unavailable.is_empty());
        assert!(outcome.pending.is_empty());
        assert_eq!(outcome.failed, paths);
        assert!(outcome.fatal);
    }
}
