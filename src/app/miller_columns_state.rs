//! State for the Miller's Columns view.
//!
//! Miller's Columns renders the full ancestor chain of `current_path` as a
//! horizontal strip of columns. The rightmost column is `current_path` and is
//! backed by the app's live `items` (full interaction stack). The ancestor
//! columns to its left need their own directory listings, which are loaded
//! here lazily on a background thread — mirroring `SidebarTreeState`, but
//! returning complete `FileEntry` listings (files + folders) sorted and
//! filtered with the same criteria the main view uses.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::application::sorting::sort_items;
use crate::domain::file_entry::{
    is_archive_extension, is_path_inside_archive, FileEntry, FoldersPosition, SortMode,
};

/// Sort/filter signature. When it changes, cached listings are invalidated so
/// every column re-sorts consistently with the active view.
type ListingSignature = (SortMode, bool, FoldersPosition, bool);

/// Result sent back from a background enumeration task.
struct LoadResult {
    dir: PathBuf,
    /// `None` = I/O error (permission denied, etc.); `Some(vec)` = success.
    entries: Option<Vec<FileEntry>>,
    /// Signature the listing was produced with (stale results are dropped).
    signature: ListingSignature,
    /// Monotonic request ID used to reject results invalidated in flight.
    request_id: u64,
}

const LOAD_FAILURE_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Cache of ancestor-column directory listings for the Miller's Columns view.
pub struct MillerColumnsState {
    listings: HashMap<PathBuf, Arc<Vec<FileEntry>>>,
    loading: HashMap<PathBuf, u64>,
    failed_at: HashMap<PathBuf, Instant>,
    next_request_id: u64,
    tx: mpsc::Sender<LoadResult>,
    rx: mpsc::Receiver<LoadResult>,
    signature: ListingSignature,
    /// Focused (rightmost) directory last rendered. When it changes (user
    /// navigated), `scroll_to_focused_pending` is set so the horizontal strip
    /// scrolls to reveal the focused column.
    focused_dir: Option<String>,
    /// One-shot request to scroll the strip to the focused column.
    scroll_to_focused_pending: bool,
    /// Persisted horizontal position, independent of egui's transient area ID.
    horizontal_scroll_offset: f32,
    /// Selection anchor for Shift interactions in an ancestor column. Paths
    /// remain stable when a cached listing is replaced or re-sorted.
    selection_anchors: HashMap<PathBuf, PathBuf>,
}

impl Clone for MillerColumnsState {
    fn clone(&self) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            listings: self.listings.clone(),
            // In-flight tasks still target the original receiver. Missing
            // listings will be scheduled again when this clone is rendered.
            loading: HashMap::new(),
            failed_at: self.failed_at.clone(),
            next_request_id: self.next_request_id,
            tx,
            rx,
            signature: self.signature,
            focused_dir: self.focused_dir.clone(),
            scroll_to_focused_pending: self.scroll_to_focused_pending,
            horizontal_scroll_offset: self.horizontal_scroll_offset,
            selection_anchors: self.selection_anchors.clone(),
        }
    }
}

impl MillerColumnsState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            listings: HashMap::new(),
            loading: HashMap::new(),
            failed_at: HashMap::new(),
            next_request_id: 0,
            tx,
            rx,
            signature: (SortMode::Name, false, FoldersPosition::First, false),
            focused_dir: None,
            scroll_to_focused_pending: false,
            horizontal_scroll_offset: 0.0,
            selection_anchors: HashMap::new(),
        }
    }

    /// Update the active sort/filter signature. Clears cached listings when it
    /// changes so subsequent `ensure()` calls reload with the new criteria.
    pub fn set_signature(&mut self, signature: ListingSignature) {
        if self.signature != signature {
            self.signature = signature;
            self.listings.clear();
            self.loading.clear();
            self.failed_at.clear();
            // In-flight tasks tagged with the old signature are dropped on poll.
        }
    }

    /// Record the currently focused (rightmost) directory. When it differs
    /// from the previous one, requests a one-shot scroll to the focused column.
    pub fn note_focused_dir(&mut self, dir: &str) {
        if self.focused_dir.as_deref() != Some(dir) {
            self.focused_dir = Some(dir.to_string());
            self.scroll_to_focused_pending = true;
        }
    }

    /// Consume the pending scroll-to-focused request (true at most once per
    /// navigation).
    pub fn take_scroll_to_focused_pending(&mut self) -> bool {
        std::mem::take(&mut self.scroll_to_focused_pending)
    }

    pub fn horizontal_scroll_offset(&self) -> f32 {
        self.horizontal_scroll_offset
    }

    pub fn set_horizontal_scroll_offset(&mut self, offset: f32) {
        self.horizontal_scroll_offset = offset.max(0.0);
    }

    pub fn selection_anchor_index(&self, directory: &Path, items: &[FileEntry]) -> Option<usize> {
        let anchor_path = self.selection_anchors.get(directory)?;
        items.iter().position(|item| &item.path == anchor_path)
    }

    pub fn set_selection_anchor(&mut self, directory: &Path, item_path: &Path) {
        self.selection_anchors
            .insert(directory.to_path_buf(), item_path.to_path_buf());
    }

    pub fn clear_selection_anchors(&mut self) {
        self.selection_anchors.clear();
    }

    /// Cached listing as a cheap-to-clone `Arc`, if already loaded.
    pub fn get_arc(&self, dir: &Path) -> Option<Arc<Vec<FileEntry>>> {
        self.listings.get(dir).cloned()
    }

    pub fn is_loading(&self, dir: &Path) -> bool {
        self.loading.contains_key(dir)
    }

    pub fn listing_contains_path(&self, directory: &Path, path: &Path) -> Option<bool> {
        self.listings
            .get(directory)
            .map(|items| items.iter().any(|item| item.path == path))
    }

    /// Ensure a directory listing is loaded (or loading) for `dir`.
    pub fn ensure(&mut self, dir: &Path) {
        if self.listings.contains_key(dir) || self.loading.contains_key(dir) {
            return;
        }
        if self
            .failed_at
            .get(dir)
            .is_some_and(|failed_at| failed_at.elapsed() < LOAD_FAILURE_RETRY_DELAY)
        {
            return;
        }
        self.start_loading(dir);
    }

    /// Invalidate a directory. A new load starts only if the directory is still
    /// part of a rendered ancestor chain and `ensure()` requests it again.
    pub fn invalidate(&mut self, dir: &Path) {
        self.listings.remove(dir);
        self.loading.remove(dir);
        self.failed_at.remove(dir);
    }

    /// Drop cached listings for directories not in `keep` to bound memory.
    pub fn retain(&mut self, keep: &HashSet<PathBuf>) {
        self.listings.retain(|dir, _| keep.contains(dir));
        self.loading.retain(|dir, _| keep.contains(dir));
        self.failed_at.retain(|dir, _| keep.contains(dir));
        self.selection_anchors
            .retain(|directory, _| keep.contains(directory));
    }

    fn start_loading(&mut self, dir: &Path) {
        let dir_buf = dir.to_path_buf();
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.loading.insert(dir_buf.clone(), request_id);
        let tx = self.tx.clone();
        let signature = self.signature;
        rayon::spawn(move || {
            let entries = enumerate_directory(&dir_buf, signature);
            let _ = tx.send(LoadResult {
                dir: dir_buf,
                entries,
                signature,
                request_id,
            });
        });
    }

    /// Drain completed background loads. Returns true if any listing changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.rx.try_recv() {
            // Drop results produced with a now-stale signature.
            if result.signature != self.signature
                || self.loading.get(&result.dir).copied() != Some(result.request_id)
            {
                continue;
            }
            self.loading.remove(&result.dir);
            if let Some(entries) = result.entries {
                self.failed_at.remove(&result.dir);
                self.listings.insert(result.dir, Arc::new(entries));
                changed = true;
            } else {
                self.failed_at.insert(result.dir, Instant::now());
            }
        }
        changed
    }
}

impl Default for MillerColumnsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Enumerate a directory into a sorted, filtered `FileEntry` listing.
/// Runs on a background thread. Returns `None` on I/O error.
fn enumerate_directory(dir: &Path, signature: ListingSignature) -> Option<Vec<FileEntry>> {
    let (mode, descending, folders_position, show_hidden) = signature;

    let is_archive_root = dir.is_file()
        && dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_archive_extension);
    if is_archive_root || is_path_inside_archive(dir) {
        let mut entries = crate::infrastructure::windows::list_shell_folder(dir).ok()?;
        if !show_hidden {
            entries.retain(|entry| !entry.is_hidden);
        }
        sort_items(&mut entries, mode, descending, folders_position);
        return Some(entries);
    }

    let read_dir = std::fs::read_dir(dir).ok()?;

    let mut entries: Vec<FileEntry> = Vec::new();
    for entry in read_dir.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();

        // Always skip system entries; skip hidden unless show_hidden is on.
        if is_system(&path) {
            continue;
        }
        let hidden = is_hidden(&path);
        if hidden && !show_hidden {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let filesystem_is_dir = file_type.is_dir();
        let is_archive = !filesystem_is_dir && is_archive_extension(&name);
        let is_dir = filesystem_is_dir || is_archive;
        let (size, modified, created) = entry
            .metadata()
            .ok()
            .map(|m| {
                let size = if filesystem_is_dir { 0 } else { m.len() };
                let modified = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let created = m
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                (size, modified, created)
            })
            .unwrap_or((0, 0, None));

        entries.push(FileEntry {
            path,
            name,
            is_dir,
            size,
            modified,
            created,
            folder_cover: None,
            drive_info: None,
            sync_status: crate::domain::file_entry::SyncStatus::None,
            is_hidden: hidden,
            recycle_bin: None,
        });
    }

    sort_items(&mut entries, mode, descending, folders_position);
    Some(entries)
}

/// Check the Windows hidden attribute.
#[cfg(windows)]
fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    std::fs::metadata(path)
        .map(|m| (m.file_attributes() & FILE_ATTRIBUTE_HIDDEN) != 0)
        .unwrap_or(false)
}

/// Check the Windows system attribute.
#[cfg(windows)]
fn is_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    std::fs::metadata(path)
        .map(|m| (m.file_attributes() & FILE_ATTRIBUTE_SYSTEM) != 0)
        .unwrap_or(false)
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
    use super::*;

    #[test]
    fn physical_archive_entries_are_navigable_and_keep_their_size() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let archive = temp.path().join("sample.zip");
        std::fs::write(&archive, b"archive placeholder").expect("create archive placeholder");

        let entries = enumerate_directory(
            temp.path(),
            (SortMode::Name, false, FoldersPosition::First, false),
        )
        .expect("enumerate directory");
        let entry = entries
            .iter()
            .find(|entry| entry.path == archive)
            .expect("archive entry");

        assert!(entry.is_dir);
        assert!(entry.is_archive());
        assert_eq!(entry.size, b"archive placeholder".len() as u64);
    }

    #[test]
    fn invalidation_rejects_an_in_flight_stale_result() {
        let mut state = MillerColumnsState::new();
        let dir = PathBuf::from(r"C:\stale");
        let request_id = state.next_request_id;
        state.next_request_id += 1;
        state.loading.insert(dir.clone(), request_id);
        state
            .tx
            .send(LoadResult {
                dir: dir.clone(),
                entries: Some(Vec::new()),
                signature: state.signature,
                request_id,
            })
            .expect("queue stale result");

        state.invalidate(&dir);

        assert!(!state.poll());
        assert!(state.get_arc(&dir).is_none());
        assert!(!state.is_loading(&dir));
    }

    #[test]
    fn selection_anchor_is_resolved_by_path_after_reordering() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let anchor = temp.path().join("anchor.txt");
        std::fs::write(&anchor, b"anchor").expect("create anchor file");
        std::fs::write(temp.path().join("other.txt"), b"other").expect("create other file");
        let mut items = enumerate_directory(
            temp.path(),
            (SortMode::Name, false, FoldersPosition::First, false),
        )
        .expect("enumerate directory");

        let mut state = MillerColumnsState::new();
        state.set_selection_anchor(temp.path(), &anchor);
        items.reverse();

        let index = state
            .selection_anchor_index(temp.path(), &items)
            .expect("resolve anchor");
        assert_eq!(items[index].path, anchor);

        state
            .listings
            .insert(temp.path().to_path_buf(), Arc::new(items.clone()));
        assert_eq!(
            state.listing_contains_path(temp.path(), &anchor),
            Some(true)
        );
        assert_eq!(
            state.listing_contains_path(temp.path(), &temp.path().join("missing.txt")),
            Some(false)
        );

        state.clear_selection_anchors();
        assert_eq!(state.selection_anchor_index(temp.path(), &items), None);
    }
}
