# Performance Optimizations — MTT File Manager

## Overview

This document describes the key performance optimizations implemented in MTT File Manager for fast directory loading, responsive filesystem monitoring, and efficient resource usage.

## 1. NtQueryDirectoryFile for Fast Directory Reading

**Location**: `src/infrastructure/ntfs_reader.rs`

For HDD drives, standard directory enumeration is slow due to seek times. The app uses `NtQueryDirectoryFile` to read directory entries in bulk (64KB per syscall), significantly reducing the number of I/O operations.

**How it works**:
- Detects storage type (SSD vs HDD) via I/O priority detection (`infrastructure/io_priority/detection.rs`)
- For HDDs, uses `NtQueryDirectoryFile` with `FileDirectoryInformation` class
- Reads 64KB of entries in a single system call
- Returns `DirectoryEntry` structs with name, size, timestamps, and attributes

**Virtual drive overrides**: The `virtual_drive_config.json` file allows manually marking drives as HDD/SSD to control which reading strategy is used.

## 2. Drive-Wide Filesystem Monitoring

**Location**: `src/infrastructure/drive_watcher.rs`, `src/infrastructure/drive_watcher/`

Instead of creating a new filesystem watcher per folder (expensive and slow during navigation), the app monitors entire drive roots.

**Architecture**:
- `DriveWatcher`: monitors a single drive root (e.g., `C:\`) using `ReadDirectoryChangesW`
- `DriveWatcherManager` (`drive_watcher_integration.rs`): manages one watcher per drive
- Async I/O with `OVERLAPPED` structure — non-blocking
- Events are filtered by the current folder prefix — only relevant changes are processed

**Benefits**:
- Zero overhead when navigating between folders (no watcher recreation)
- Instant change detection for the current folder
- Single watcher per drive regardless of navigation depth

**Fallback**: UNC/network paths use the `notify` crate (`notify-watcher` feature) since drive-root monitoring doesn't apply to network shares.

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
- **High priority**: Visible thumbnail generation, current folder loading
- **Low priority**: Prefetch, background warmup, folder size calculation

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
