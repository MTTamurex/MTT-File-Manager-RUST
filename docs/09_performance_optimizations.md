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

**Why this is the default**: The drive-wide `ReadDirectoryChangesW` watcher (see below) was disabled by default because recursive monitoring of drive roots causes systemic UI degradation on machines with OneDrive/Cloud Files minifilters over prolonged use.

### Opt-In: Drive-Wide Watcher (ReadDirectoryChangesW)

**Location**: `src/infrastructure/drive_watcher.rs`, `src/infrastructure/drive_watcher/`

**Activation**: Set `MTT_ENABLE_DRIVE_WATCHER=1` environment variable.

When enabled, the app monitors entire drive roots instead of individual folders:
- `DriveWatcher`: monitors a single drive root (e.g., `C:\`) using `ReadDirectoryChangesW`
- `DriveWatcherManager` (`drive_watcher_integration.rs`): manages one watcher per drive
- Async I/O with `OVERLAPPED` structure — non-blocking
- Events are filtered by the current folder prefix — only relevant changes are processed
- On NTFS/ReFS drives, the `notify` watcher is dropped entirely when the drive watcher is active

**Benefits when enabled**:
- Zero overhead when navigating between folders on the same drive (no watcher recreation)
- Instant change detection for the current folder
- Single watcher per drive regardless of navigation depth

### Resilience: Consistency Probe

**Location**: `src/app/init_workers/consistency_probe_worker.rs`

A background worker periodically computes a directory listing signature and compares it against the disk. This catches events that any watcher might miss (common on non-NTFS/non-USN filesystems). Unreliable drives are escalated to active polling mode.

### UNC/Network Paths

UNC and network paths always use the `notify` crate since drive-root monitoring doesn't apply to network shares.

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

## 4. Thumbnail Pipeline Optimization

The 5-stage thumbnail pipeline is designed for maximum hit rate with minimal overhead:

1. **Stage 1 (image crate)**: Fastest path — pure Rust, no COM initialization
2. **Stage 2 (WIC)**: Windows Imaging Component — handles formats not supported by image crate
3. **Stage 3 (Shell API)**: IShellItemImageFactory — handles Shell-specific formats
4. **Stage 4 (Force extract)**: Forced extraction for edge cases
5. **Stage 5 (Media Foundation)**: Video thumbnails via frame extraction

Each stage only runs if the previous one fails, minimizing expensive COM/Shell calls.

**Thumbnail compression**: Generated thumbnails are compressed to WebP format for smaller disk cache footprint.

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
- Window radius = 6 (up to 13 images cached simultaneously)
- 512MB memory budget with eviction by distance from current position
- Workers check an `AtomicUsize` center before decoding — obsolete jobs are skipped
- Navigation requests only the new edge image (tail-only), not the full window
- Bounded channels prevent infinite job accumulation

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

On hybrid GPU laptops (Intel + NVIDIA/AMD), Windows may route GUI-subsystem apps to the integrated GPU. The app forces discrete GPU selection via:
- **NVIDIA**: `NvOptimusEnablement = 1` (exported static)
- **AMD**: `AmdPowerXpressRequestHighPerformance = 1` (exported static)
- **wgpu**: `PowerPreference::HighPerformance` in `WgpuConfiguration`

### DPI Awareness

The `app.manifest` (embedded via `build.rs` + `winresource`) declares **Per-Monitor V2 DPI awareness**. This prevents DWM from bitmap-scaling the window on high-DPI displays, avoiding blurriness and GPU overhead.

## 12. Restore-from-Idle Optimization

**Location**: `src/ui/app/lifecycle.rs`, `src/ui/app_impl.rs`

When the app returns from an idle or minimized state:

- **GPU texture flush**: Only flushes textures after 60s of idle (prevents unnecessary VRAM churn on short idle periods)
- **Burst mode**: Short burst window (`2s + idle_secs/120`, capped at 5s) with aggressive `frame_time_peak_ms` decay (0.50 factor) to prevent inflated peak metrics from starving thumbnail upload budgets
- **No watcher throttling**: Watcher event batches are not reduced after restore, ensuring filesystem changes are processed at full speed
