//! Icon loading functionality for the file manager.
//!
//! This module handles loading Windows shell icons for files and folders.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use eframe::egui;
use lru::LruCache;

extern crate windows as windows_crate;

use crate::domain::file_entry::IconSize;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::windows;

mod async_ops;
mod file_icons;
mod special_icons;

/// RAII guard for Single-Threaded Apartment COM initialization on icon
/// extraction threads. Required by `SHParseDisplayName` /
/// `IShellItemImageFactory` to resolve PIDL-based icons correctly.
///
/// Behavior:
/// - On success: schedules `CoUninitialize` in `Drop` (balanced).
/// - On `RPC_E_CHANGED_MODE`: COM was previously initialized as MTA on this
///   thread; we do NOT call `CoUninitialize` (we did not init), and shell
///   icons may degrade to generic. Logged at debug level.
/// - On any other failure: logged at warn level for diagnostics.
///
/// Using a guard ensures `CoUninitialize` is invoked even if the worker
/// closure panics, preventing per-thread COM leaks.
struct ComStaGuard {
    needs_uninit: bool,
}

impl ComStaGuard {
    fn new() -> Self {
        use windows_crate::Win32::Foundation::RPC_E_CHANGED_MODE;
        use windows_crate::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        // SAFETY: CoInitializeEx is balanced by CoUninitialize in Drop when
        // `needs_uninit` is true. The HRESULT is inspected to distinguish
        // success from "already initialized in different mode".
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_ok() {
            Self { needs_uninit: true }
        } else if hr == RPC_E_CHANGED_MODE {
            log::debug!(
                "[Icon] COM already initialized as MTA on this thread (RPC_E_CHANGED_MODE); \
                 shell icons may fall back to generic"
            );
            Self {
                needs_uninit: false,
            }
        } else {
            log::warn!(
                "[Icon] CoInitializeEx(STA) failed: HRESULT 0x{:08X} — \
                 shell icons may fall back to generic",
                hr.0 as u32
            );
            Self {
                needs_uninit: false,
            }
        }
    }
}

impl Drop for ComStaGuard {
    fn drop(&mut self) {
        if self.needs_uninit {
            // SAFETY: paired with the successful CoInitializeEx in `new`.
            unsafe {
                ::windows::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

/// Result from a background icon extraction thread.
struct AsyncIconResult {
    key: String,
    data: Option<(Vec<u8>, u32, u32)>,
}

const DRIVE_ICON_CACHE_CAPACITY: usize = 64;
const FAILED_DRIVE_ICON_CAPACITY: usize = 256;
const EXTENSION_ICON_CACHE_CAPACITY: usize = 512;

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
    drive_icon_cache: LruCache<String, egui::TextureHandle>,
    /// Remember failed drive/shell icon attempts to avoid retrying every frame
    failed_drive_icons: LruCache<String, ()>,
    /// Cache for extension-based icons (extension -> texture)
    pub extension_cache: LruCache<String, egui::TextureHandle>,
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
    /// Optional SQLite disk cache for persisting shell icons across sessions.
    disk_cache: Option<Arc<ThumbnailDiskCache>>,
}

impl Default for IconLoader {
    fn default() -> Self {
        Self::new(None)
    }
}

impl IconLoader {
    /// Creates a new icon loader.
    ///
    /// When `disk_cache` is provided, shell icons (special folders, drives,
    /// computer, recycle bin) will be persisted to SQLite and loaded
    /// instantly on subsequent launches.
    pub fn new(disk_cache: Option<Arc<ThumbnailDiskCache>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            icon_cache: LruCache::new(
                NonZeroUsize::new(512).expect("icon cache size must be non-zero"),
            ),
            folder_icon_texture: None,
            computer_icon_texture: None,
            recycle_bin_icon_texture: None,
            drive_icon_cache: LruCache::new(
                NonZeroUsize::new(DRIVE_ICON_CACHE_CAPACITY)
                    .expect("drive icon cache size must be non-zero"),
            ),
            failed_drive_icons: LruCache::new(
                NonZeroUsize::new(FAILED_DRIVE_ICON_CAPACITY)
                    .expect("failed drive icon cache size must be non-zero"),
            ),
            extension_cache: LruCache::new(
                NonZeroUsize::new(EXTENSION_ICON_CACHE_CAPACITY)
                    .expect("extension icon cache size must be non-zero"),
            ),
            loading_drive_icons: HashSet::new(),
            icon_result_rx: rx,
            icon_result_tx: tx,
            sync_icon_budget_window_start: Instant::now(),
            sync_icon_budget_elapsed: Duration::ZERO,
            sync_icon_budget_calls: 0,
            disk_cache,
        }
    }

    /// Clears all icon caches (preserves folder_icon_texture since it's a static composed graphic).
    pub fn clear(&mut self) {
        self.icon_cache.clear();
        self.drive_icon_cache.clear();
        self.failed_drive_icons.clear();
        self.extension_cache.clear();
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

    pub fn cache_counts(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.icon_cache.len(),
            self.extension_cache.len(),
            self.drive_icon_cache.len(),
            self.failed_drive_icons.len(),
            self.loading_drive_icons.len(),
        )
    }

    /// Set of Jumbo icon cache keys currently being loaded in background.
    /// Tracks file icon Jumbo extractions (separate from drive/folder icons).
    pub fn is_jumbo_icon_loading(&self, cache_key: &str) -> bool {
        self.loading_drive_icons.contains(cache_key)
    }

    /// Enqueue an asynchronous Jumbo icon extraction for the preview panel.
    ///
    /// Spawns a background thread to extract the icon, sends the result
    /// through the async icon channel, and stores it in `icon_cache` (keyed
    /// with `_Jumbo` suffix) when `poll_async_icons` picks it up.
    pub fn enqueue_jumbo_icon(&mut self, path: &std::path::Path, is_virtual: bool) {
        let path_text = path.to_string_lossy();
        let cache_key = format!("{}_Jumbo", path_text);

        // Already in-flight or previously failed — skip.
        if self.loading_drive_icons.contains(&cache_key)
            || self.failed_drive_icons.peek(&cache_key).is_some()
        {
            return;
        }

        self.loading_drive_icons.insert(cache_key.clone());
        let path_owned = path.to_path_buf();
        let tx = self.icon_result_tx.clone();
        std::thread::spawn(move || {
            // STA COM is required for SHParseDisplayName / IShellItemImageFactory
            // to correctly resolve PIDL-based icons (especially ZIP virtual paths).
            // Without explicit init, Shell API may auto-init as MTA and return
            // generic icons. The guard logs failures and ensures balanced
            // CoUninitialize even on panic.
            let _com = ComStaGuard::new();
            let data = if is_virtual {
                windows::extract_shell_icon(&path_owned, IconSize::Jumbo)
                    .map_err(|e| {
                        log::trace!(
                            "[Icon] Shell icon extraction failed for {:?}: {}",
                            path_owned,
                            e
                        )
                    })
                    .ok()
            } else {
                windows::extract_file_icon_by_path(&path_owned, IconSize::Jumbo)
                    .map_err(|e| {
                        log::trace!(
                            "[Icon] File icon extraction failed for {:?}: {}",
                            path_owned,
                            e
                        )
                    })
                    .ok()
            };
            let _ = tx.send(AsyncIconResult {
                key: cache_key,
                data,
            });
        });
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
        self.sync_icon_budget_elapsed = self.sync_icon_budget_elapsed.saturating_add(elapsed);
    }
}
