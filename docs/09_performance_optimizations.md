# Performance Optimizations — MTT File Manager

## Overview

This document describes the key performance optimizations implemented in MTT File Manager for fast directory loading, responsive filesystem monitoring, and efficient resource usage.

## 1. NtQueryDirectoryFile for Fast Directory Reading

**Location**: `src/infrastructure/ntfs_reader.rs`

The app uses `NtQueryDirectoryFile` to read directory entries in bulk (64KB per syscall), significantly reducing the number of I/O operations compared to standard enumeration.

**How it works**:
- Detects storage type (SSD vs HDD) via `infrastructure/io_priority/detection.rs` (`is_ssd()` function)
- Uses `NtQueryDirectoryFile` with `FileDirectoryInformation` class for directory reading
- Reads 64KB of entries in a single system call
- Returns `DirectoryEntry` structs with name, size, timestamps, and attributes
- A separate `hdd_directory_reader.rs` provides `read_directory_hdd_batched()` for optimized HDD reads

**Virtual drive overrides**: The `%LOCALAPPDATA%\MTT-File-Manager\virtual_drive_config.json` file is created automatically on first launch, pre-populated with detected virtual drives, and allows the user to mark them as HDD/SSD to control which reading strategy is used.

## 2. Filesystem Monitoring Strategy

### Default: Per-Folder Watcher (`notify` crate)

**Location**: `src/app/operations/watcher.rs`

By default, the app uses the `notify` crate (`RecommendedWatcher`) to monitor the **current folder only** (non-recursive). When the user navigates to a different folder, the previous watcher is dropped and a new one is created.

### Resilience: Consistency Probe

**Location**: `src/app/init_workers/consistency_probe_worker.rs`

A background worker periodically computes a directory listing signature and compares it against the disk. This catches events that any watcher might miss (common on non-NTFS/non-USN filesystems). Unreliable drives are escalated to active polling mode.

When the probe detects a visible folder-cover change on a non-NTFS drive, the UI now invalidates the corresponding folder-size caches together with the cover/preview state. This keeps the list-view `Size` column and the details panel aligned with preview changes instead of letting stale folder-size values survive until app restart.

### UNC/Network Paths

UNC and network paths use the `notify` crate with the consistency probe as a resilience backup.

## 3. Smart DELETE Handling

When a file is deleted, the watcher receives a DELETE event. Instead of reloading the entire folder (expensive), the app removes the deleted item directly from the UI list.

```
DELETE event received
    ↓
Match event path against current folder items
    ↓
Remove matching item from the items list
    ↓
UI updates immediately — no folder reload needed
```

This eliminates unnecessary I/O and keeps the UI responsive during batch deletes.

## 3.5 Folder Size Fast Path and Cancellation

**Locations**: `src/app/folder_size_state.rs`, `src/app/init_workers/filesystem_workers.rs`, `src/infrastructure/global_search.rs`

Folder-size reads use a dual-path strategy:
- **NTFS fast path**: `SearchRequest::FolderSize` goes over Named Pipe IPC to `mtt-search-service`, which computes folder totals from its in-memory MFT/USN index with zero disk I/O in the app process.
- **Fallback path**: non-NTFS volumes, or NTFS when IPC is unavailable, use `calculate_folder_size_parallel()` (`FindFirstFileExW` + rayon).

List-view folder sizes are handled by a dedicated batch worker with:
- `batch_cancel` to abort slow scans on navigation or List→Grid transitions
- `batch_generation` to discard queued requests from the previous folder
- per-request invalidation epochs (`BatchSizeRequest` / `BatchSizeResult`) so stale in-flight results are rejected instead of re-populating caches after a folder changed
- deferred revalidation for NTFS because the search service applies USN changes on a 2-second incremental loop, so an immediate IPC request can still observe the old total briefly

## 4. Thumbnail Pipeline Optimization

The 5-stage thumbnail pipeline is designed for maximum hit rate with minimal overhead:

1. **Stage 1 (image crate)**: Fastest path — pure Rust, no COM initialization
2. **Stage 2 (WIC)**: Windows Imaging Component — handles formats not supported by image crate
3. **Stage 3 (Shell API)**: IShellItemImageFactory — handles Shell-specific formats
4. **Stage 4 (Force extract)**: Forced extraction for edge cases
5. **Stage 5 (Media Foundation)**: Video thumbnails via frame extraction

Each stage only runs if the previous one fails, minimizing expensive COM/Shell calls.

**Thumbnail compression**: Generated thumbnails are compressed to WebP format for smaller disk cache footprint.

**GPU texture upload budgeting**: The app tracks frame time and adjusts `upload_budget_ms` (clamped 2.0–10.0 ms) dynamically. During scrolling or video playback, the budget is reduced further (60–85% of normal) to prevent UI stutter.

## 5. Custom Folder Cover Composition

**Location**: `src/infrastructure/folder_compose.rs`

Replaces Windows Shell API folder cover generation entirely. Composes folder previews from 3 embedded PNG layers:

1. `folder_back_512.png` — folder silhouette (background)
2. Content thumbnail — first image/video found inside the folder
3. `folder_front_512.png` — folder tab overlay (foreground)

**Performance**: ~1-2ms per composition vs 20-200ms via Shell API COM calls. PNGs are embedded via `include_bytes!` and decoded once during startup (~2ms total).

Results are cached in SQLite (`folder_previews` table) with invalidation based on folder content modification time.

## 6. I/O Priority Management

**Location**: `src/infrastructure/io_priority.rs`, `src/infrastructure/io_priority/`

Worker threads adjust their I/O priority based on workload type:
- **Interactive** (priority 0): Visible thumbnail generation, current folder loading — user is waiting
- **Prefetch** (priority 1, default): Thumbnails that will be visible soon
- **Background** (priority 2): Folder covers, metadata discovery, warmup

Uses `ThreadPriorityGuard` for RAII-based priority restoration.

## 7. Sliding-Window Image Cache

**Location**: `src/image_viewer/cache.rs`

The dedicated image viewer uses a sliding-window cache strategy:
- Window radius = 1 (current image + immediate neighbors)
- Cache stores GPU `TextureHandle`s instead of CPU-side decoded RGBA frames
- Large images are capped to `DISPLAY_CACHE_MAX_SIDE = 4096` before entering the viewer cache
- Workers check an `AtomicUsize` center before decoding — obsolete jobs are skipped
- Navigation requests only the new edge image (tail-only), not the full window
- Bounded channels prevent infinite job accumulation

The viewer startup path was also simplified:
- The image viewer no longer seeds its first frame from the file-manager thumbnail cache, avoiding wrong-zoom startup artifacts and a second full-frame decode being skipped accidentally.
- The root viewport starts hidden and is revealed after the first viewer frame is ready, which keeps startup transitions cleaner.

## 7.5 Standalone Viewer Runtime Baseline

**Location**: `src/viewer_runtime.rs`

Image, PDF, and text viewers are launched as separate processes from the same executable (`--image-viewer`, `--pdf-viewer`, `--text-viewer`). Their shared runtime is intentionally lighter than the main app:

- Locale and theme are read via a tiny read-only SQLite query instead of the full `AppStateDb` initialization path.
- `eframe::Renderer::Glow` is used for viewers instead of `Wgpu`.
- `persist_window` is disabled.
- `multisampling`, `depth_buffer`, and `stencil_buffer` are all disabled for viewer windows.

This lowered the common baseline RSS for text, PDF, and image viewers significantly compared with reusing the main app's Wgpu-heavy startup profile.

## 8. Adaptive Batch Loading

**Location**: `src/infrastructure/adaptive_batch.rs`

Folder loading uses adaptive batch sizes that adjust based on system performance. Large folders are loaded in batches to keep the UI responsive, with batch sizes tuning automatically.

## 9. Directory Cache & Prefetch

- **Directory cache** (`infrastructure/directory_cache.rs`): In-memory cache of directory structures for instant back/forward navigation
- **Prefetch worker** (`workers/prefetch_worker.rs`): Pre-loads adjacent directories during idle time
- **Idle warmup** (`workers/idle_warmup.rs`): Warms caches during idle periods

## 10. UI Virtualization

Grid and list views only render items that are currently visible in the viewport. Combined with scroll prediction, this allows smooth scrolling through folders with thousands of files.

## 11. GPU Selection & DPI Awareness

**Location**: `src/main.rs`, `app.manifest`

### GPU Preference

The main window uses `Wgpu` with `PowerPreference::HighPerformance` and honors the saved backend preference (`dx12`, `vulkan`, `gl`, `auto`).

The process deliberately does **not** export the legacy `NvOptimusEnablement` / `AmdPowerXpressRequestHighPerformance` symbols anymore. Because the same executable is reused for the standalone viewers, forcing the discrete GPU at process start would inflate baseline RAM/VRAM even for simple text/PDF/image viewing.

Standalone viewers therefore use the lighter `Glow` path from `viewer_runtime.rs`, while the main file-manager window keeps the higher-throughput `Wgpu` path.

### DPI Awareness

The `app.manifest` (embedded via `build.rs` + `winresource`) declares **Per-Monitor V2 DPI awareness**. This prevents DWM from bitmap-scaling the window on high-DPI displays, avoiding blurriness and GPU overhead.

## 12. Restore-from-Idle Optimization

**Location**: `src/ui/app/lifecycle.rs`, `src/ui/app_impl.rs`

When the app returns from an idle or minimized state:

- **GPU texture flush**: Only flushes textures after 60s of idle (prevents unnecessary VRAM churn on short idle periods)
- **Burst mode**: Short burst window (`2s + idle_secs/120`, capped at 5s) with aggressive `frame_time_peak_ms` decay (0.50 factor) to prevent inflated peak metrics from starving thumbnail upload budgets
- **No watcher throttling**: Watcher event batches are not reduced after restore, ensuring filesystem changes are processed at full speed

## 13. Archive Extraction Optimization

**Location**: `src/infrastructure/archive_extract.rs`

Native archive extraction includes several performance and safety optimizations:
- **Pre-scan**: ZIP, 7z, and RAR handlers pre-scan the central directory to count matching entries before extraction, enabling accurate progress bars
- **Streaming**: TAR variants use streaming decompression without loading the entire archive into memory
- **Cancellation**: All extraction paths check an `AtomicBool` cancel flag and abort mid-operation
- **Path sanitization**: `sanitize_relative_path()` strips `.` and `..` components to prevent Zip Slip attacks
- **Reserved name filtering**: `sanitize_filename()` rewrites Windows reserved device names (CON, PRN, AUX, NUL, COM0-9, LPT0-9)

## 14. Search Service Performance

**Location**: `crates/mtt-search-service/`

The search service optimizes indexing and query performance through several mechanisms:
- **Binary snapshots**: `index_<drive>.bin` provides a fast-start cache that loads directly into memory without SQLite parsing overhead
- **Name arena**: Lowered strings stored in a contiguous `NameArena` enable SIMD-accelerated search via `memchr`
- **USN incremental loop**: 2-second catch-up cycles minimize re-scanning on NTFS volumes
- **Per-volume threading**: Each volume gets its own indexer thread, parallelizing startup
- **Dirty-shutdown detection**: The `service_meta.dirty` flag skips expensive FTS5 rebuilds on clean shutdowns

## 15. Startup Time Optimizations

**Location**: `src/main.rs`, `src/app/init.rs`

- **Async font loading**: Custom fonts (Segoe UI) are loaded in a background thread so the window appears immediately with default fonts
- **Hidden viewport start**: The main window starts hidden (`with_visible(false)`) and is revealed after the first frame is ready, preventing visual flicker
- **eframe storage cleanup**: Stale eframe RON storage is removed before startup to prevent truncation-related hangs
- **DLL search hardening**: `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)` removes the current working directory from the DLL search order, preventing DLL planting attacks
- **GPU backend pre-read**: The `gpu_backend` preference is read from SQLite before eframe initialization to avoid renderer restart
