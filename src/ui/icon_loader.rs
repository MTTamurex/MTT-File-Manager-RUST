//! Icon loading functionality for the file manager.
//!
//! This module handles loading Windows shell icons for files and folders.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::mpsc;

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
    extension_cache: HashMap<String, egui::TextureHandle>,
    /// Keys currently being loaded in background threads (prevents duplicate requests)
    loading_drive_icons: HashSet<String>,
    /// Channel to receive completed icon extractions from background threads
    icon_result_rx: mpsc::Receiver<AsyncIconResult>,
    /// Sender cloned into background threads
    icon_result_tx: mpsc::Sender<AsyncIconResult>,
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
        }
    }

    /// Clears all icon caches.
    pub fn clear(&mut self) {
        self.icon_cache.clear();
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
        self.folder_icon_texture = None;
        self.computer_icon_texture = None;
    }

    /// Clears drive icon caches (both successful and failed), allowing fresh extraction.
    /// Called when device events indicate drive insertion/removal.
    pub fn clear_drive_icons(&mut self) {
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
    }
}
