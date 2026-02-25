//! Icon loading functionality for the file manager.
//!
//! This module handles loading Windows shell icons for files and folders.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use lru::LruCache;

use crate::domain::file_entry::IconSize;
use crate::infrastructure::windows;

mod async_ops;
mod file_icons;
mod special_icons;

/// Result from a background icon extraction thread.
struct AsyncIconResult {
    key: String,
    data: Option<(Vec<u8>, u32, u32)>,
}

/// Manages loading and caching of Windows shell icons.
pub struct IconLoader {
    /// Cache for file icons (path -> texture)
    pub icon_cache: LruCache<String, egui::TextureHandle>,
    /// Folder icon texture (cached)
    folder_icon_texture: Option<egui::TextureHandle>,
    /// Computer icon texture (cached)
    computer_icon_texture: Option<egui::TextureHandle>,
    /// Recycle bin icon texture (cached)
    recycle_bin_icon_texture: Option<egui::TextureHandle>,
    /// Drive icon cache (drive path -> texture)
    drive_icon_cache: HashMap<String, egui::TextureHandle>,
    /// Remember failed drive/shell icon attempts to avoid retrying every frame
    failed_drive_icons: HashSet<String>,
    /// Cache for extension-based icons (extension -> texture)
    pub extension_cache: HashMap<String, egui::TextureHandle>,
    /// Keys currently being loaded in background threads (prevents duplicate requests)
    loading_drive_icons: HashSet<String>,
    /// Channel to receive completed icon extractions from background threads
    icon_result_rx: mpsc::Receiver<AsyncIconResult>,
    /// Sender cloned into background threads
    icon_result_tx: mpsc::Sender<AsyncIconResult>,
    /// Per-frame budget guard for non-blocking icon lookups that still hit Windows Shell.
    sync_icon_budget_window_start: Instant,
    sync_icon_budget_elapsed: Duration,
    sync_icon_budget_calls: usize,
}

impl Default for IconLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl IconLoader {
    /// Creates a new icon loader.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            icon_cache: LruCache::new(
                NonZeroUsize::new(512).expect("icon cache size must be non-zero"),
            ),
            folder_icon_texture: None,
            computer_icon_texture: None,
            recycle_bin_icon_texture: None,
            drive_icon_cache: HashMap::new(),
            failed_drive_icons: HashSet::new(),
            extension_cache: HashMap::new(),
            loading_drive_icons: HashSet::new(),
            icon_result_rx: rx,
            icon_result_tx: tx,
            sync_icon_budget_window_start: Instant::now(),
            sync_icon_budget_elapsed: Duration::ZERO,
            sync_icon_budget_calls: 0,
        }
    }

    /// Clears all icon caches (preserves folder_icon_texture since it's a static composed graphic).
    pub fn clear(&mut self) {
        self.icon_cache.clear();
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
        // NOTE: folder_icon_texture is NOT cleared — it's a static custom composed
        // graphic set once at startup (back+front+paper_sheet layers).
        self.computer_icon_texture = None;
        self.sync_icon_budget_window_start = Instant::now();
        self.sync_icon_budget_elapsed = Duration::ZERO;
        self.sync_icon_budget_calls = 0;
    }

    /// Clears drive icon caches (both successful and failed), allowing fresh extraction.
    /// Called when device events indicate drive insertion/removal.
    pub fn clear_drive_icons(&mut self) {
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
    }

    fn can_run_non_blocking_sync_icon_lookup(
        &mut self,
        path: &std::path::Path,
        allow_blocking: bool,
    ) -> bool {
        if allow_blocking {
            return true;
        }

        // Never run sync shell icon lookups in UI for OneDrive paths.
        // OneDrive shell/metadata calls may stall for hundreds of ms.
        if crate::infrastructure::onedrive::is_onedrive_path(path) {
            return false;
        }

        const WINDOW: Duration = Duration::from_millis(16);
        const MAX_CALLS_PER_WINDOW: usize = 6;
        const MAX_TIME_PER_WINDOW: Duration = Duration::from_millis(4);

        if self.sync_icon_budget_window_start.elapsed() >= WINDOW {
            self.sync_icon_budget_window_start = Instant::now();
            self.sync_icon_budget_elapsed = Duration::ZERO;
            self.sync_icon_budget_calls = 0;
        }

        self.sync_icon_budget_calls < MAX_CALLS_PER_WINDOW
            && self.sync_icon_budget_elapsed < MAX_TIME_PER_WINDOW
    }

    fn record_non_blocking_sync_icon_lookup(&mut self, elapsed: Duration, allow_blocking: bool) {
        if allow_blocking {
            return;
        }
        self.sync_icon_budget_calls = self.sync_icon_budget_calls.saturating_add(1);
        self.sync_icon_budget_elapsed =
            self.sync_icon_budget_elapsed.saturating_add(elapsed);
    }
}
