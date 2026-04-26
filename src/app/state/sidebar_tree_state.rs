use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::infrastructure::directory_cache::DirectoryCache;

/// A single folder node in the sidebar tree.
#[derive(Clone, Debug, PartialEq)]
pub struct FolderNode {
    pub path: PathBuf,
    pub name: String,
    /// `None` = not yet probed, `Some(true)` = has subdirectories (show expand arrow).
    pub has_subfolders: Option<bool>,
    /// Whether this folder has the Windows FILE_ATTRIBUTE_HIDDEN flag.
    pub is_hidden: bool,
}

/// Result sent back from the background loading thread.
struct LoadResult {
    parent: PathBuf,
    /// `None` = I/O error (permission denied, etc.); `Some(vec)` = successful enumeration.
    children: Option<Vec<FolderNode>>,
}

/// State for the hierarchical folder tree displayed in the sidebar.
pub struct SidebarTreeState {
    /// Paths the user has expanded (or that were auto-expanded by navigation sync).
    expanded: HashSet<PathBuf>,
    /// Cached children for each expanded directory.
    children: HashMap<PathBuf, Vec<FolderNode>>,
    /// Directories currently being loaded in a background thread.
    loading: HashSet<PathBuf>,
    /// Channel sender for background results.
    tx: mpsc::Sender<LoadResult>,
    /// Channel receiver for background results (drained each frame).
    rx: mpsc::Receiver<LoadResult>,
    /// Shared directory cache from the central panel — avoids duplicate read_dir.
    dir_cache: Arc<DirectoryCache>,
    /// Whether hidden files/folders are currently shown.
    show_hidden: bool,
    /// Smooth scroll: the target offset (where the user wants to go).
    pub scroll_target_y: f32,
    /// Smooth scroll: the visual offset (smoothly animates toward target).
    pub scroll_visual_y: f32,
    /// Last time we refreshed all expanded directories to catch external changes.
    last_full_refresh: Instant,
}

impl SidebarTreeState {
    pub fn new(dir_cache: Arc<DirectoryCache>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            expanded: HashSet::new(),
            children: HashMap::new(),
            loading: HashSet::new(),
            tx,
            rx,
            dir_cache,
            show_hidden: false,
            scroll_target_y: 0.0,
            scroll_visual_y: 0.0,
            last_full_refresh: Instant::now(),
        }
    }

    // ── Queries ──────────────────────────────────────────────────────

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub fn is_loading(&self, path: &Path) -> bool {
        self.loading.contains(path)
    }

    pub fn get_children(&self, path: &Path) -> Option<&[FolderNode]> {
        self.children.get(path).map(|v| v.as_slice())
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Snapshot the per-tab sidebar state (expanded nodes + scroll position).
    /// Used by `sync_to_tab()` to persist sidebar state per tab.
    pub fn snapshot_expanded(&self) -> HashSet<PathBuf> {
        self.expanded.clone()
    }

    /// Return the current scroll target (for per-tab persistence).
    pub fn snapshot_scroll_y(&self) -> f32 {
        self.scroll_target_y
    }

    /// Restore per-tab sidebar state (expanded nodes + scroll position).
    /// Used by `sync_from_tab()` when switching to a tab.
    /// Children for newly expanded nodes are loaded on demand; children
    /// for nodes that are no longer expanded are kept in cache for reuse.
    pub fn restore_expanded(&mut self, expanded: HashSet<PathBuf>, scroll_y: f32) {
        self.expanded = expanded;
        self.scroll_target_y = scroll_y;
        self.scroll_visual_y = scroll_y;

        // Ensure children are loaded for all expanded directories.
        // If they're already cached, this is a no-op; otherwise
        // a background load is triggered.
        let to_load: Vec<PathBuf> = self
            .expanded
            .iter()
            .filter(|p| {
                !self.children.contains_key(p.as_path()) && !self.loading.contains(p.as_path())
            })
            .cloned()
            .collect();
        for path in to_load {
            self.load_children(&path);
        }
    }

    /// Update show_hidden flag. If changed, invalidates all cached children
    /// so they are re-loaded with the new filter.
    pub fn set_show_hidden(&mut self, show: bool) {
        if self.show_hidden != show {
            self.show_hidden = show;
            // Re-load all currently expanded directories
            let expanded: Vec<PathBuf> = self.expanded.iter().cloned().collect();
            self.children.clear();
            for path in expanded {
                if !self.loading.contains(&path) {
                    self.load_children(&path);
                }
            }
        }
    }

    // ── Mutations ────────────────────────────────────────────────────

    /// Toggle expand/collapse. If expanding and no cached children, starts loading.
    pub fn toggle_expand(&mut self, path: &Path) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_path_buf());
            if !self.children.contains_key(path) && !self.loading.contains(path) {
                self.load_children(path);
            }
        }
    }

    /// Invalidate cached children for a directory (e.g. on file system change).
    /// If the directory is currently expanded, re-triggers loading.
    pub fn clear_children(&mut self, path: &Path) {
        self.children.remove(path);
        if self.expanded.contains(path) && !self.loading.contains(path) {
            self.load_children(path);
        }
    }

    /// Remove all state for a given drive root (e.g. when a USB drive is removed).
    pub fn clear_drive(&mut self, drive_root: &Path) {
        let drive_prefix = drive_root.to_path_buf();
        self.expanded.retain(|p| !p.starts_with(&drive_prefix));
        self.children.retain(|p, _| !p.starts_with(&drive_prefix));
        self.loading.retain(|p| !p.starts_with(&drive_prefix));
    }

    /// Periodically re-enumerates all expanded directories to detect external
    /// changes (e.g. folders created/deleted by other applications).
    ///
    /// The per-folder `notify` watcher only watches the content panel's current
    /// directory, so sidebar-expanded paths elsewhere won't receive events.
    /// This method bridges that gap with a lightweight periodic check.
    ///
    /// Fires background re-enumeration for all expanded directories.
    /// Does NOT request repaint — `poll_loaded()` handles that only when data
    /// actually changed, avoiding unnecessary repaint churn.
    pub fn refresh_expanded_if_stale(&mut self) {
        const REFRESH_INTERVAL_SECS: u64 = 5;

        if self.expanded.is_empty()
            || self.last_full_refresh.elapsed().as_secs() < REFRESH_INTERVAL_SECS
        {
            return;
        }
        self.last_full_refresh = Instant::now();

        // Invalidate the DirectoryCache so the background thread does a real
        // re-enumeration instead of replaying stale cache entries.
        for path in &self.expanded {
            self.dir_cache.invalidate(path);
        }

        // Fire background loads for all expanded directories.
        // Do NOT remove existing children — they stay visible until `poll_loaded()`
        // atomically replaces them with the fresh result (zero flicker).
        let expanded: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        for path in &expanded {
            if !self.loading.contains(path) {
                self.start_loading(path);
            }
        }
    }

    // ── Background Loading ───────────────────────────────────────────

    /// Try the shared directory cache first (instant); fall back to background thread.
    fn load_children(&mut self, path: &Path) {
        let path_buf = path.to_path_buf();
        let show_hidden = self.show_hidden;
        // Fast path: reuse the central panel's directory cache
        if let Some(entries) = self.dir_cache.get(&path_buf) {
            let children = entries_to_folder_nodes(&entries, show_hidden);
            // Simulate a completed load — update in place
            let has_children = !children.is_empty();
            self.children.insert(path_buf.clone(), children);
            if !has_children {
                if let Some(grandparent) = path_buf.parent() {
                    if let Some(siblings) = self.children.get_mut(grandparent) {
                        if let Some(node) = siblings.iter_mut().find(|n| n.path == path_buf) {
                            node.has_subfolders = Some(false);
                        }
                    }
                }
                self.expanded.remove(&path_buf);
            }
            return;
        }
        // Slow path: background thread
        self.start_loading(path);
    }

    /// Queue a background task (on rayon's thread pool) to enumerate subdirectories.
    fn start_loading(&mut self, path: &Path) {
        self.loading.insert(path.to_path_buf());
        let parent = path.to_path_buf();
        let tx = self.tx.clone();
        let show_hidden = self.show_hidden;

        rayon::spawn(move || {
            let children = enumerate_subfolders(&parent, show_hidden);
            let _ = tx.send(LoadResult { parent, children });
        });
    }

    /// Drain all completed background loads. Call once per frame from the update loop.
    pub fn poll_loaded(&mut self) -> bool {
        let mut any = false;
        while let Ok(result) = self.rx.try_recv() {
            self.loading.remove(&result.parent);

            match result.children {
                Some(children) => {
                    let has_children = !children.is_empty();

                    // Only update + repaint if children actually changed.
                    let changed = self.children.get(&result.parent) != Some(&children);
                    if !changed {
                        continue;
                    }

                    self.children.insert(result.parent.clone(), children);

                    // Update the parent's has_subfolders in its own parent's children list
                    // so the arrow disappears for empty folders.
                    if !has_children {
                        if let Some(grandparent) = result.parent.parent() {
                            if let Some(siblings) = self.children.get_mut(grandparent) {
                                if let Some(node) =
                                    siblings.iter_mut().find(|n| n.path == result.parent)
                                {
                                    node.has_subfolders = Some(false);
                                }
                            }
                        }
                        // Also collapse it since there's nothing to show
                        self.expanded.remove(&result.parent);
                    }
                }
                None => {
                    // I/O error — collapse the node but don't mark has_subfolders=false
                    // so the arrow stays and the user can retry later.
                    self.expanded.remove(&result.parent);
                }
            }

            any = true;
        }
        any
    }
}

// ── Folder Enumeration (runs on background thread) ───────────────────

/// Convert cached FileEntry list into FolderNode list (synchronous, no I/O).
fn entries_to_folder_nodes(
    entries: &[crate::domain::file_entry::FileEntry],
    show_hidden: bool,
) -> Vec<FolderNode> {
    let mut folders: Vec<FolderNode> = entries
        .iter()
        // The content panel treats archive files as navigable entries, but the
        // sidebar tree should only ever show real filesystem directories.
        .filter(|e| e.is_dir && !e.is_archive())
        .filter(|e| !is_system(&e.path))
        .filter(|e| show_hidden || !is_hidden(&e.path))
        .map(|e| FolderNode {
            path: e.path.clone(),
            name: e.name.clone(),
            has_subfolders: None,
            is_hidden: is_hidden(&e.path),
        })
        .collect();
    folders.sort_by(|a, b| natord::compare_ignore_case(&a.name, &b.name));
    folders
}

/// List immediate subdirectories of `parent`, sorted alphabetically.
/// Returns `None` on I/O error (permission denied, etc.).
fn enumerate_subfolders(parent: &Path, show_hidden: bool) -> Option<Vec<FolderNode>> {
    let read_dir = match std::fs::read_dir(parent) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("sidebar-tree: cannot read {}: {}", parent.display(), e);
            return None;
        }
    };

    let mut folders: Vec<FolderNode> = Vec::new();

    for entry in read_dir.flatten() {
        // Only include directories
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        // Always skip system folders
        if is_system(&path) {
            continue;
        }

        let hidden = is_hidden(&path);
        // Skip hidden folders unless show_hidden is enabled
        if hidden && !show_hidden {
            continue;
        }

        folders.push(FolderNode {
            path,
            name,
            has_subfolders: None,
            is_hidden: hidden,
        });
    }

    folders.sort_by(|a, b| natord::compare_ignore_case(&a.name, &b.name));

    Some(folders)
}

/// Check Windows hidden attribute only.
#[cfg(windows)]
fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    match std::fs::metadata(path) {
        Ok(meta) => (meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN) != 0,
        Err(_) => true,
    }
}

/// Check Windows system attribute only.
#[cfg(windows)]
fn is_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    match std::fs::metadata(path) {
        Ok(meta) => (meta.file_attributes() & FILE_ATTRIBUTE_SYSTEM) != 0,
        Err(_) => true,
    }
}

#[cfg(not(windows))]
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_system(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::entries_to_folder_nodes;
    use crate::domain::file_entry::FileEntry;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn entries_to_folder_nodes_excludes_archive_entries_from_cache_projection() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("mtt-sidebar-tree-test-{unique}"));
        fs::create_dir_all(&base).expect("create temp base dir");

        let real_dir = base.join("Folder");
        fs::create_dir_all(&real_dir).expect("create real subdir");

        let archive = base.join("archive.zip");
        File::create(&archive).expect("create archive file");

        let entries = vec![
            FileEntry::from_path(real_dir.clone(), true),
            FileEntry::from_path(archive.clone(), true),
        ];

        let nodes = entries_to_folder_nodes(&entries, true);
        let node_paths: Vec<PathBuf> = nodes.into_iter().map(|node| node.path).collect();

        assert!(node_paths.contains(&real_dir));
        assert!(!node_paths.contains(&archive));

        let _ = fs::remove_file(&archive);
        let _ = fs::remove_dir_all(&base);
    }
}
