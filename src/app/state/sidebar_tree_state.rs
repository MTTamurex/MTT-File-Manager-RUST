use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// A single folder node in the sidebar tree.
#[derive(Clone, Debug)]
pub struct FolderNode {
    pub path: PathBuf,
    pub name: String,
    /// `None` = not yet probed, `Some(true)` = has subdirectories (show expand arrow).
    pub has_subfolders: Option<bool>,
}

/// Result sent back from the background loading thread.
struct LoadResult {
    parent: PathBuf,
    children: Vec<FolderNode>,
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
    /// The last path we auto-scrolled to (prevents re-scrolling every frame).
    pub last_synced_path: Option<PathBuf>,
}

impl SidebarTreeState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            expanded: HashSet::new(),
            children: HashMap::new(),
            loading: HashSet::new(),
            tx,
            rx,
            last_synced_path: None,
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

    // ── Mutations ────────────────────────────────────────────────────

    /// Toggle expand/collapse. If expanding and no cached children, starts loading.
    pub fn toggle_expand(&mut self, path: &Path) {
        if self.expanded.contains(path) {
            self.expanded.remove(path);
        } else {
            self.expanded.insert(path.to_path_buf());
            if !self.children.contains_key(path) && !self.loading.contains(path) {
                self.start_loading(path);
            }
        }
    }

    /// Expand all ancestors of `target` so the tree reveals it.
    /// Returns paths that need loading (not yet cached).
    pub fn expand_to_path(&mut self, target: &Path) {
        let mut ancestors: Vec<PathBuf> = Vec::new();
        let mut current = target.to_path_buf();

        // Collect all ancestors up to the drive root
        while let Some(parent) = current.parent() {
            // Stop at drive root (e.g. C:\)
            if parent == current {
                break;
            }
            ancestors.push(parent.to_path_buf());
            current = parent.to_path_buf();
        }

        // Expand from root toward leaf
        for ancestor in ancestors.iter().rev() {
            if !self.expanded.contains(ancestor) {
                self.expanded.insert(ancestor.clone());
            }
            if !self.children.contains_key(ancestor) && !self.loading.contains(ancestor) {
                self.start_loading(ancestor);
            }
        }
    }

    /// Invalidate cached children for a directory (e.g. on file system change).
    /// If the directory is currently expanded, re-triggers loading.
    pub fn clear_children(&mut self, path: &Path) {
        self.children.remove(path);
        if self.expanded.contains(path) && !self.loading.contains(path) {
            self.start_loading(path);
        }
    }

    /// Remove all state for a given drive root (e.g. when a USB drive is removed).
    pub fn clear_drive(&mut self, drive_root: &Path) {
        let drive_prefix = drive_root.to_path_buf();
        self.expanded.retain(|p| !p.starts_with(&drive_prefix));
        self.children.retain(|p, _| !p.starts_with(&drive_prefix));
        self.loading.retain(|p| !p.starts_with(&drive_prefix));
    }

    // ── Background Loading ───────────────────────────────────────────

    /// Spawn a background thread to enumerate subdirectories of `path`.
    fn start_loading(&mut self, path: &Path) {
        self.loading.insert(path.to_path_buf());
        let parent = path.to_path_buf();
        let tx = self.tx.clone();

        std::thread::Builder::new()
            .name("sidebar-tree-load".into())
            .spawn(move || {
                let children = enumerate_subfolders(&parent);
                let _ = tx.send(LoadResult {
                    parent,
                    children,
                });
            })
            .ok();
    }

    /// Drain all completed background loads. Call once per frame from the update loop.
    pub fn poll_loaded(&mut self) -> bool {
        let mut any = false;
        while let Ok(result) = self.rx.try_recv() {
            self.loading.remove(&result.parent);
            self.children.insert(result.parent, result.children);
            any = true;
        }
        any
    }
}

// ── Folder Enumeration (runs on background thread) ───────────────────

/// List immediate subdirectories of `parent`, sorted alphabetically.
/// For each child, peeks one level deeper to determine `has_subfolders`.
fn enumerate_subfolders(parent: &Path) -> Vec<FolderNode> {
    let read_dir = match std::fs::read_dir(parent) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
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

        // Skip hidden/system folders (Windows attribute check)
        if is_hidden_or_system(&path) {
            continue;
        }

        // Peek one level deeper: does this folder have any subdirectories?
        let has_subfolders = peek_has_subfolders(&path);

        folders.push(FolderNode {
            path,
            name,
            has_subfolders: Some(has_subfolders),
        });
    }

    // Sort case-insensitive
    folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    folders
}

/// Quick check: does `dir` contain at least one visible subdirectory?
fn peek_has_subfolders(dir: &Path) -> bool {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return false,
    };

    for entry in read_dir.flatten() {
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() && !is_hidden_or_system(&entry.path()) {
                return true;
            }
        }
    }

    false
}

/// Check Windows hidden/system attributes.
#[cfg(windows)]
fn is_hidden_or_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

    match std::fs::metadata(path) {
        Ok(meta) => {
            let attrs = meta.file_attributes();
            (attrs & FILE_ATTRIBUTE_HIDDEN) != 0 || (attrs & FILE_ATTRIBUTE_SYSTEM) != 0
        }
        Err(_) => true, // Treat inaccessible as hidden
    }
}

#[cfg(not(windows))]
fn is_hidden_or_system(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}
