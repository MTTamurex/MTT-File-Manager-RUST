//! Path-scoped exclusion between deletion and optional folder-size work.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

#[derive(Default)]
pub struct DeleteActivity(Mutex<State>);

#[derive(Default)]
struct State {
    next_id: u64,
    deletions: HashMap<u64, Vec<PathBuf>>,
    roots: HashMap<PathBuf, usize>,
    ancestors: HashMap<PathBuf, usize>,
    scans: HashMap<u64, (PathBuf, Weak<AtomicBool>)>,
}

fn key(path: &Path) -> PathBuf {
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let text = text
        .strip_prefix(r"\\?\unc\")
        .map(|s| format!(r"\\{s}"))
        .unwrap_or_else(|| text.strip_prefix(r"\\?\").unwrap_or(&text).to_string());
    PathBuf::from(text)
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

impl State {
    fn affected(&self, path: &Path) -> bool {
        self.ancestors.contains_key(path)
            || path
                .ancestors()
                .any(|ancestor| self.roots.contains_key(ancestor))
    }

    fn next(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

impl DeleteActivity {
    pub fn active_count(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .deletions
            .len()
    }

    /// True only for deletion roots and their descendants, not aggregate ancestors.
    pub fn contains(&self, path: &Path) -> bool {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        key(path)
            .ancestors()
            .any(|ancestor| state.roots.contains_key(ancestor))
    }

    pub fn affects(&self, path: &Path) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .affected(&key(path))
    }

    pub fn begin(self: &Arc<Self>, roots: &[PathBuf]) -> DeletionGuard {
        let roots: Vec<_> = roots.iter().map(|path| key(path)).collect();
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        for (path, cancel) in state.scans.values() {
            if roots.iter().any(|root| overlaps(path, root)) {
                if let Some(cancel) = cancel.upgrade() {
                    cancel.store(true, Ordering::Release);
                }
            }
        }
        let id = state.next();
        for root in &roots {
            *state.roots.entry(root.clone()).or_default() += 1;
            for ancestor in root.ancestors() {
                *state.ancestors.entry(ancestor.to_path_buf()).or_default() += 1;
            }
        }
        state.deletions.insert(id, roots);
        DeletionGuard {
            activity: self.clone(),
            id,
        }
    }

    /// Register only the currently executing scan, never an entire worker queue.
    /// The same lock closes the race with a deletion starting after this check.
    pub fn scan(self: &Arc<Self>, path: &Path, cancel: &Arc<AtomicBool>) -> Option<ScanGuard> {
        let path = key(path);
        let mut state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.affected(&path) {
            return None;
        }
        let id = state.next();
        state.scans.insert(id, (path, Arc::downgrade(cancel)));
        Some(ScanGuard {
            activity: self.clone(),
            id,
        })
    }
}

pub struct DeletionGuard {
    activity: Arc<DeleteActivity>,
    id: u64,
}

impl Drop for DeletionGuard {
    fn drop(&mut self) {
        let mut state = self.activity.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(roots) = state.deletions.remove(&self.id) {
            for root in roots {
                decrement(&mut state.roots, &root);
                for ancestor in root.ancestors() {
                    decrement(&mut state.ancestors, ancestor);
                }
            }
        }
    }
}

fn decrement(counts: &mut HashMap<PathBuf, usize>, path: &Path) {
    if let Some(count) = counts.get_mut(path) {
        *count -= 1;
        if *count == 0 {
            counts.remove(path);
        }
    }
}

pub struct ScanGuard {
    activity: Arc<DeleteActivity>,
    id: u64,
}

impl Drop for ScanGuard {
    fn drop(&mut self) {
        self.activity
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .scans
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspension_is_scoped_and_survives_overlapping_operations() {
        let activity = Arc::new(DeleteActivity::default());
        let first = activity.begin(&[PathBuf::from(r"C:\Data\Folder")]);
        let second = activity.begin(&[PathBuf::from(r"C:\Data\Folder\Child")]);
        for path in [r"C:\Data", r"c:\data\FOLDER", r"\\?\C:\Data\Folder\Nested"] {
            assert!(activity.affects(Path::new(path)));
        }
        for path in [r"C:\Data\Folder-other", r"C:\Other", r"D:\Data"] {
            assert!(!activity.affects(Path::new(path)));
        }
        drop(first);
        assert!(activity.affects(Path::new(r"C:\Data\Folder")));
        drop(second);
        assert!(!activity.affects(Path::new(r"C:\Data")));
    }

    #[test]
    fn cancels_only_registered_affected_scans_and_rejects_queued_work() {
        let activity = Arc::new(DeleteActivity::default());
        let affected = Arc::new(AtomicBool::new(false));
        let independent = Arc::new(AtomicBool::new(false));
        let _scan = activity.scan(Path::new(r"C:\Data"), &affected).unwrap();
        let _other = activity.scan(Path::new(r"D:\Other"), &independent).unwrap();
        let deletion = activity.begin(&[PathBuf::from(r"C:\Data\Folder")]);
        assert!(affected.load(Ordering::Acquire));
        assert!(!independent.load(Ordering::Acquire));
        assert!(activity.scan(Path::new(r"C:\Data"), &affected).is_none());
        drop(deletion);
        assert!(activity.scan(Path::new(r"C:\Data"), &affected).is_some());
    }

    #[test]
    fn finished_scan_does_not_cancel_an_unrelated_next_request() {
        let activity = Arc::new(DeleteActivity::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let scan = activity.scan(Path::new(r"C:\Data"), &cancel).unwrap();
        drop(scan);
        let _next = activity.scan(Path::new(r"D:\Other"), &cancel).unwrap();
        let _deletion = activity.begin(&[PathBuf::from(r"C:\Data\Folder")]);
        assert!(!cancel.load(Ordering::Acquire));
    }

    #[test]
    fn network_and_verbatim_paths_share_the_same_scope() {
        let activity = Arc::new(DeleteActivity::default());
        let _deletion = activity.begin(&[PathBuf::from(r"\\?\UNC\server\share\Folder")]);
        assert!(activity.affects(Path::new(r"\\SERVER\share\folder\Nested")));
        assert!(activity.affects(Path::new(r"\\server\share")));
        assert!(!activity.affects(Path::new(r"\\server\other")));
        assert!(activity.contains(Path::new(r"\\server\share\Folder")));
        assert!(!activity.contains(Path::new(r"\\server\share")));
    }
}
