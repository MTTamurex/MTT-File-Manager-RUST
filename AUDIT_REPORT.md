# MTT File Manager - Validated Audit Report And Fix Handoff

Updated: 2026-06-06

This report is the source of truth for the next agent starting fixes. It replaces the older audit findings with validated, reclassified, and prioritized items checked against the current codebase.

## 1. Executive Summary

Project snapshot:

| Metric | Current value |
|---|---:|
| Tracked Rust files | 387 |
| Tracked Rust LOC | 104,543 |
| Rust files over 500 lines in `src/` | 46 |
| Rust files over 1000 lines in `src/` | 4 |
| `ImageViewerApp` public fields | about 212 |

Highest priority risks:

| Priority | Area | Why it matters |
|---|---|---|
| P0 | `HBITMAP` cleanup after `hbitmap_to_rgba(...) ?` | Leaks GDI handles on conversion failure. |
| P0 | Search tooltip metadata reads on UI thread | Can freeze UI on OneDrive/cloud-only files. |
| P0 | Search tooltip SQLite + WebP decode on UI thread | Causes frame stutter and blocks render path. |
| P1 | `ext_key_stack()` long-extension panic | Release build can panic on long extensions. |
| P1 | Duplicate `hbitmap_to_rgba` without dimension guards | Potential huge allocation on invalid/corrupt bitmap. |
| P1 | `DirectoryCache::put()` full key rebuild | Avoidable O(n) work inside mutex during navigation. |

Items intentionally reclassified or removed:

| Original claim | Current verdict |
|---|---|
| Icon loader is broadly thread-per-request | Overstated. Main icon worker is a bounded 2-4 thread pool; only auxiliary drive/folder/jumbo paths spawn per request. |
| `GetLastError()` diagnostics are proven wrong | Not confirmed. Current code captures `GetLastError()` immediately after `DeviceIoControl().is_err()`. |
| `metadata::image::is_image_extension()` is dead code | False. It is called from `src/infrastructure/windows/metadata/mod.rs:101`. |
| `ext_key_stack()` is stack buffer overflow | False as memory corruption. It is a release panic risk in safe Rust. |

Recommended fix order:

1. Fix P0 resource leaks and UI-thread blocking first.
2. Fix P1 robustness/performance issues next.
3. Only then handle low-impact cleanup and architecture debt.
4. Do not spend time on invalidated findings unless new evidence appears.

## 2. P0 Fixes

### P0-01: GDI `HBITMAP` Leak On Error Path

Status: validated.

Impact: high. If `hbitmap_to_rgba(hbitmap)?` returns `Err`, `DeleteObject(hbitmap)` is skipped and the process leaks a GDI handle. GDI handles are limited per process and leaks can degrade Windows rendering over long sessions.

Affected owned `HBITMAP` call sites:

| File | Lines | Current pattern |
|---|---:|---|
| `src/workers/thumbnail/extraction/stage3_shell_api.rs` | 66-67 | `hbitmap_to_rgba(hbitmap)?` before `DeleteObject`. |
| `src/infrastructure/windows/icons/thumbnails.rs` | 27-30 | Same pattern. |
| `src/infrastructure/windows/icons/thumbnails.rs` | 64-66 | Same pattern. |
| `src/infrastructure/windows/icons/thumbnails.rs` | 72-74 | Same pattern. |
| `src/infrastructure/windows/icons/special.rs` | 86-90 | Same pattern. |
| `src/infrastructure/windows/icons/special.rs` | 158-162 | Same pattern. |
| `src/infrastructure/windows/icons/file_icons.rs` | 87-91 | Same pattern. |
| `src/infrastructure/windows/icons/file_icons.rs` | 157-161 | Same pattern. |
| `src/infrastructure/windows/icons/file_icons.rs` | 227-231 | Same pattern. |

Important ownership note:

Do not call `DeleteObject` for `HBITMAP` obtained from `ISharedBitmap::GetSharedBitmap()` in `src/infrastructure/windows/icons/thumbnails.rs`. Ownership remains with `ISharedBitmap` there.

Minimal fix pattern:

```rust
let result = crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap);
let _ = DeleteObject(hbitmap.into());
let (rgba_data, width, height) = result?;
```

If the code returns the tuple directly, use:

```rust
let result = crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap);
let _ = DeleteObject(hbitmap.into());
Ok(result?)
```

Suggested verification:

| Command | Purpose |
|---|---|
| `cargo check` | Ensure ownership/error propagation still compiles. |
| Manual Windows smoke test | Browse folders with mixed file types and shell icons. |

### P0-02: Blocking `std::fs::metadata()` In Search Tooltip UI Path

Status: validated.

Impact: high/critical. `std::fs::metadata()` can block for a long time on OneDrive cloud-only files. The codebase already documents this risk in `src/ui/app/panels/content.rs:338-341`, but the global search tooltip still does synchronous metadata reads.

Affected code:

| File | Lines | Problem |
|---|---:|---|
| `src/ui/global_search_overlay/result_row.rs` | 331-362 | Two `std::fs::metadata(&full_path)` calls during hover tooltip rendering. |

Required behavior:

| Current | Target |
|---|---|
| Tooltip blocks while reading metadata. | Tooltip renders immediately with cached/partial data. |
| Metadata cache miss performs filesystem I/O on UI thread. | Cache miss schedules background metadata load. |
| UI waits for size/date. | UI requests repaint when worker result arrives. |

Implementation guidance:

1. Add a small async tooltip metadata request path to global search state or reuse an existing metadata/live-size worker if it fits without coupling unrelated flows.
2. Track in-flight paths to avoid sending one request per frame while hovered.
3. On cache miss, show unknown size/date or omit those rows.
4. On worker result, update `app.global_search.metadata_cache` and `size_cache`, then request repaint.
5. Avoid `FileEntry::from_path()` or any equivalent helper that calls `std::fs::metadata()` on the UI thread.

Suggested verification:

| Command/test | Purpose |
|---|---|
| `cargo check` | Validate state/channel changes. |
| Hover search results in OneDrive folder | Tooltip must appear without freezing. |
| Hover same result repeatedly | Must not enqueue unbounded duplicate requests. |

### P0-03: SQLite Read + WebP Decode + Texture Upload In Search Tooltip

Status: validated.

Impact: high. The tooltip can perform SQLite I/O, WebP decode, and texture creation during render, causing frame stutter and UI stalls.

Affected code:

| File | Lines | Problem |
|---|---:|---|
| `src/ui/global_search_overlay/result_row.rs` | 381-396 | `app.disk_cache.get_latest(&p)`, `image::load_from_memory_with_format(... WebP)`, and `load_texture(...)` in tooltip path. |

Required behavior:

| Current | Target |
|---|---|
| Disk cache read on UI thread. | Disk cache read happens in background. |
| WebP decode on UI thread. | Decode happens in background. |
| Texture upload happens immediately on tooltip cache miss. | UI uploads only ready decoded RGBA with a small per-frame budget or reuses existing texture cache. |

Implementation guidance:

1. Add a tooltip thumbnail async state keyed by full path.
2. Background worker should load from `ThumbnailDiskCache`, decode WebP to RGBA, and send `(path, rgba, width, height)`.
3. UI should store final `TextureHandle` in `app.global_search.tooltip_texture_cache` after receiving decoded data.
4. Keep tooltip rendering non-blocking; show no thumbnail or a placeholder while loading.
5. Deduplicate in-flight thumbnail requests.

Suggested verification:

| Command/test | Purpose |
|---|---|
| `cargo check` | Validate channel/state changes. |
| Hover many media search results | No obvious frame hitch or repeated decode. |
| Hover non-media search results | No thumbnail request should be made. |

## 3. P1 Fixes

### P1-01: `ext_key_stack()` Release Panic On Long Extensions

Status: validated with corrected classification.

Impact: medium. This is not a stack buffer overflow, but it can panic in release for long extensions because `debug_assert!` is stripped.

Affected code:

| File | Lines |
|---|---:|
| `src/ui/icon_loader/file_icons.rs` | 8-18 |

Problem:

```rust
let len = ext_str.len() + suffix.len();
debug_assert!(len <= 32, "ext key too long for stack buffer");
buf[..ext_str.len()].copy_from_slice(ext_str.as_bytes());
buf[ext_str.len()..len].copy_from_slice(suffix.as_bytes());
```

Minimal safe fix options:

| Option | Tradeoff |
|---|---|
| Runtime guard plus heap fallback | Preserves stack fast path for normal extensions and handles rare long extensions. |
| Always allocate a `String` key | Simpler, but removes the optimized hot path. |

Recommended fix:

Use stack fast path when `len <= 32`; otherwise build a `String` and route callers through a helper returning `Cow<'_, str>` or split helper paths clearly. Do not truncate silently unless cache-key collisions are impossible by construction.

Suggested tests:

| Test | Purpose |
|---|---|
| Long extension over 32 bytes | Must not panic. |
| Normal extension | Existing cache lookup behavior preserved. |
| Canonical shared extension path | Cache key remains stable. |

### P1-02: Duplicate `hbitmap_to_rgba` Without Dimension Validation

Status: validated.

Impact: medium. `stage3_shell_api.rs` has a private conversion function without the dimension guards present in the shared implementation.

Affected code:

| File | Lines |
|---|---:|
| `src/workers/thumbnail/extraction/stage3_shell_api.rs` | 80-129 |
| `src/infrastructure/windows/bitmap_conversion.rs` | 25-27 shared guard |

Required fix:

Remove the duplicate private function and call `crate::infrastructure::windows::bitmap_conversion::hbitmap_to_rgba(hbitmap)`.

Order dependency:

This can be done together with P0-01 for `stage3_shell_api.rs`, because that file needs both cleanup ordering and conversion deduplication.

Suggested verification:

| Command | Purpose |
|---|---|
| `cargo check` | Ensure imports and call sites compile. |

### P1-03: `DirectoryCache::put()` Rebuilds Ordered Keys And Recomputes Total Items

Status: validated, but original fix was incomplete.

Impact: medium. Current code rebuilds `ordered_keys` from all LRU entries and computes `total_items()` by iterating all cached folders under the mutex.

Affected code:

| File | Lines | Problem |
|---|---:|---|
| `src/infrastructure/directory_cache.rs` | 35-36 | `sync_ordered_keys()` rebuilds the full `BTreeSet`. |
| `src/infrastructure/directory_cache.rs` | 104 | Rebuild after oversized folder rejection. |
| `src/infrastructure/directory_cache.rs` | 116 | Full `total_items()` sum. |
| `src/infrastructure/directory_cache.rs` | 125 | Rebuild after `put()`. |

Important correctness note:

Do not just delete `sync_ordered_keys()`. `invalidate_children()` depends on `ordered_keys`, so `put()` must maintain it incrementally.

Required fix shape:

1. Add `total_items: usize` to `DirectoryCacheInner`.
2. On `put(path, entries)`, subtract old entry length if replacing an existing key.
3. Insert the new key into `ordered_keys` when the folder is cached.
4. If `LruCache::put()` evicts an entry, remove the evicted path from `ordered_keys` and subtract its item count.
5. On oversized folder rejection, remove that path from both `entries` and `ordered_keys`, and subtract any removed length.
6. On `invalidate`, `invalidate_children`, and `clear`, keep `total_items` in sync.
7. Keep tests for `invalidate_children` and add tests for replacement, oversized rejection, and LRU eviction.

Suggested verification:

| Command | Purpose |
|---|---|
| `cargo test directory_cache` | Validate cache bookkeeping. |
| `cargo check` | Validate integration. |

### P1-04: Thread Spawn Failures Are Silently Discarded

Status: validated.

Impact: medium for folder load, low-medium for GIF, low for folder preview.

Affected code:

| File | Lines | Impact |
|---|---:|---|
| `src/app/operations/folder_loading/load_pipeline.rs` | 38-40 | Primary navigation can silently fail. |
| `src/ui/components/gif_manager.rs` | 105-124 | GIF decode pool may have fewer/no workers with no diagnostic. |
| `src/workers/folder_preview_worker.rs` | 184-187 | Folder preview worker may be missing silently. |

Required fix:

| Location | Minimum behavior |
|---|---|
| Folder load pipeline | Log spawn error and send a failure response or empty result for the current generation so UI does not remain stuck. |
| GIF manager | Log per-worker spawn error. Consider continuing if at least one worker spawned; log if zero workers spawned. |
| Folder preview worker | Log spawn error. If practical, return a `Result` from `spawn_folder_preview_worker` and handle it at caller. |

Suggested verification:

| Command | Purpose |
|---|---|
| `cargo check` | Validate function signatures if changed. |

### P1-05: `DirectoryIndex` Uses `prepare()` For Repeated Queries

Status: validated.

Impact: low-medium. Simple performance improvement.

Affected code:

| File | Lines |
|---|---:|
| `src/infrastructure/directory_index.rs` | 87, 135 |

Required fix:

Use `prepare_cached()` for repeated read queries in `get_directory()` and `try_get_directory()`.

Suggested verification:

| Command | Purpose |
|---|---|
| `cargo check` | Validate rusqlite usage. |

## 4. P2 Cleanup And Lower Priority Improvements

### P2-01: `std::sync::Mutex` In SQLite Cache Modules

Status: partially valid, low impact.

Affected files:

| File | Current use |
|---|---|
| `src/infrastructure/disk_cache.rs` | `Arc<Mutex<Connection>>` for reader/writer. |
| `src/infrastructure/directory_index.rs` | `Mutex<Connection>`. |
| `src/infrastructure/icon_disk_cache.rs` | `Mutex<Connection>` and `Mutex<()>`. |

Do not treat this as a production bug. Switching to `parking_lot::Mutex` is fine for consistency and small lock overhead reduction, but it changes poisoning behavior and affects code using `lock().ok()?`.

Recommendation:

Only do this after P0/P1 items. If changed, update all lock error paths intentionally rather than mechanically.

### P2-02: Auxiliary Icon Loader Spawns Are Unbounded

Status: partially valid, lower scope than original report.

Main icon loading is already bounded:

| File | Evidence |
|---|---|
| `src/app/init_workers/visual_workers.rs:126` | Bounded crossbeam fanout channel. |
| `src/app/init_workers/visual_workers.rs:153-155` | `worker_count = cpu.clamp(2, 4)`. |
| `src/app/operations/thumbnails.rs:359-388` | `request_icon_load()` sends to worker. |

Auxiliary unbounded spawn paths remain:

| File | Line | Flow |
|---|---:|---|
| `src/ui/icon_loader/async_ops.rs` | 73 | Drive icon extraction. |
| `src/ui/icon_loader/async_ops.rs` | 111 | Folder path icon extraction. |
| `src/ui/icon_loader.rs` | 235 | Jumbo preview icon extraction. |

Recommendation:

Do not rewrite the main icon pipeline. If fixing this, consolidate only the auxiliary spawn paths into a small bounded worker or shared queue.

### P2-03: MPV Event Loop Join Timeout Is Tight

Status: valid, low impact.

Affected code:

| File | Lines |
|---|---:|
| `src/ui/components/mpv/event_loop.rs` | 218-224 |

Current loop sleeps 250 ms and shutdown waits 300 ms. Increase the wait to 500 ms to cover two poll cycles.

### P2-04: `catch_unwind(AssertUnwindSafe)` Around Mute Toggle

Status: partially valid, low impact.

Affected code:

| File | Lines |
|---|---:|
| `src/ui/components/mpv_preview/playback_state.rs` | 100-102 |
| `src/ui/components/media_preview.rs` | 143-146 |

The previously cited `src/ui/components/mpv_preview/controls.rs` file does not exist in the current tree.

Recommendation:

Either remove the wrappers or keep them with logging. Do not spend time on this before P0/P1 fixes.

### P2-05: Unbounded `mpsc::channel()` In `IconLoader`

Status: valid, low impact.

Affected code:

| File | Line |
|---|---:|
| `src/ui/icon_loader.rs` | 128 |

Risk is low because results are deduplicated by key and `poll_async_icons()` limits GPU uploads per frame. A bounded channel with fallback/drop behavior is more defensive but not urgent.

## 5. Architecture Debt

These are validated, but they should not block P0/P1 bug fixes.

### ARCH-01: `ImageViewerApp` God Struct

Status: validated and worse than the old report stated.

Affected code:

| File | Lines | Current state |
|---|---:|---|
| `src/app/state/mod.rs` | 73-509 | About 212 public fields. |

Potential extraction targets:

| Candidate | Reason |
|---|---|
| `ThumbnailPipelineState` | Queue, receivers, pending uploads, generation, cache tuning. |
| `WatcherState` | Notify watcher, device events, fallback polling, probes. |
| `IconWorkerState` | Icon channels, in-flight sets, failed icon caches. |
| `DragDropState` | Drag payload, hovered/target folder, cross-panel fields. |
| `PerformanceState` | Frame time, upload budget, memory maintenance, restore burst. |

Do not attempt broad struct extraction as part of small bug fixes unless needed by the specific fix.

### ARCH-02: Domain Depends On Infrastructure

Status: validated and incomplete in old report.

Affected dependencies:

| File | Dependency |
|---|---|
| `src/domain/file_entry.rs:1` | `DriveType` from `infrastructure::windows::system_info`. |
| `src/domain/file_entry.rs:105` | `is_media_extension` from `infrastructure::windows`. |
| `src/domain/thumbnail.rs:1` | `IOPriority` from `infrastructure::io_priority`. |
| `src/domain/errors.rs:10` | `SecurityError` from `infrastructure::security`. |

Recommendation:

Fix incrementally. Moving `DriveType` and `IOPriority` into `domain` is reasonable, but it has broad compile impact. Do not mix with P0 UI/resource fixes.

### ARCH-03: App Layer Imports UI Types

Status: validated.

Examples in `src/app/state/mod.rs`:

| Line | UI dependency |
|---:|---|
| 21 | `FxHashSet` re-exported by `ui::cache`. |
| 45 | `MediaPreview`. |
| 46 | `ContextMenuState`. |
| 47 | `IconLoader`. |
| 48 | `SvgIconManager`. |
| 132 | `CacheManager`. |
| 169-170 | `RectangleSelectionState`. |
| 187 | `GifPlayer`. |
| 322 | `GifManager`. |
| 412-413 | `PendingOperations`, `ScrollPredictor`. |

Recommendation:

Treat as architecture debt. Do not refactor broadly before fixing P0/P1.

### ARCH-04: Large Files

Status: validated with updated metrics.

Current largest files:

| Lines | File |
|---:|---|
| 1443 | `src/app/operations/message_handler/thumbnail_uploads.rs` |
| 1328 | `src/app/state/helpers.rs` |
| 1167 | `src/ui/cache.rs` |
| 1047 | `src/infrastructure/archive_extract.rs` |
| 986 | `src/workers/thumbnail/queue.rs` |
| 918 | `src/ui/app/panels/content.rs` |
| 906 | `src/app/operations/message_handler/helpers.rs` |
| 901 | `src/ui/sidebar.rs` |
| 873 | `src/image_viewer/app/mod.rs` |
| 858 | `src/ui/toolbar.rs` |
| 828 | `src/video_player/mod.rs` |
| 799 | `src/infrastructure/diagnostic_logger.rs` |

Recommendation:

Only split files when touching a coherent responsibility for a bug fix. Avoid speculative broad modularization.

## 6. Dead Code And Safe Removals

Validated dead or deprecated wrappers:

| Item | Location | Current verdict |
|---|---|---|
| `filter_items()` | `src/application/mod.rs:24-29` | No exact callers found. |
| `filter_items_opt()` | `src/application/mod.rs:33-38` | No exact callers found. |
| `filter_items_cow()` | `src/application/mod.rs:41-46` | No exact callers found. |
| `format_date()` | `src/ui/views/common.rs:45-47` | No exact callers found. |
| `format_size()` | `src/ui/views/common.rs:50-52` | No exact callers found. |
| `delete_with_shell()` | `src/application/file_operations.rs:109-119` | Deprecated, no exact callers found. |
| `rename_with_shell()` | `src/application/file_operations.rs:133-152` | Deprecated, no exact callers found. |

Not dead code:

| Item | Reason |
|---|---|
| `src/infrastructure/windows/metadata/image.rs::is_image_extension()` | Called by `src/infrastructure/windows/metadata/mod.rs:101`. It is redundant, but not dead. |

Recommendation:

Remove validated dead wrappers only after P0/P1 fixes or when doing a cleanup-only pass.

## 7. Invalidated Or Deprioritized Old Findings

### ERR-04: `GetLastError()` Captured Unreliably

Current verdict: not confirmed.

Locations exist:

| File | Lines |
|---|---:|
| `crates/mtt-search-service/src/mft_reader.rs` | 207-218, 471-482 |
| `crates/mtt-search-service/src/usn_journal.rs` | 189-200, 282-305 |

Reason for reclassification:

The code captures `GetLastError()` immediately after `DeviceIoControl(...).is_err()`. No intervening Win32 call was found before the error capture. Refactoring to extract the code from the `windows-rs` error may still be cleaner, but this should not be treated as a proven bug or high priority.

### Icon Loader Thread Explosion

Current verdict: partially valid only for auxiliary paths.

Do not rewrite the bounded main icon worker. See P2-02 for the limited remaining scope.

### `ext_key_stack()` Stack Overflow

Current verdict: wrong failure mode.

The issue is release panic on slice bounds, not memory corruption.

## 8. Suggested Fix Batches For Next Agent

### Batch 1: Resource Safety

Scope:

| Fix | Files |
|---|---|
| P0-01 `HBITMAP` cleanup | `stage3_shell_api.rs`, `icons/thumbnails.rs`, `icons/special.rs`, `icons/file_icons.rs`. |
| P1-02 shared bitmap conversion | `stage3_shell_api.rs`. |

Verification:

| Command/test | Purpose |
|---|---|
| `cargo check` | Compile after import/removal changes. |
| Manual browse test | Exercise shell thumbnails/icons. |

### Batch 2: Search Tooltip Non-Blocking UI

Scope:

| Fix | Files likely involved |
|---|---|
| P0-02 async tooltip metadata | `result_row.rs`, global search state, possibly init/update handlers. |
| P0-03 async tooltip thumbnail decode | `result_row.rs`, global search state, worker/channel setup. |

Verification:

| Command/test | Purpose |
|---|---|
| `cargo check` | Compile channel/state changes. |
| Hover OneDrive result | No freeze on cloud-only files. |
| Hover media result | Thumbnail appears after async load without stutter. |

### Batch 3: Robustness And Cache Perf

Scope:

| Fix | Files |
|---|---|
| P1-01 long extension guard | `src/ui/icon_loader/file_icons.rs`. |
| P1-03 incremental `DirectoryCache` bookkeeping | `src/infrastructure/directory_cache.rs`. |
| P1-05 `prepare_cached()` | `src/infrastructure/directory_index.rs`. |

Verification:

| Command/test | Purpose |
|---|---|
| `cargo test directory_cache` | Validate cache key/total bookkeeping. |
| Long extension unit test | Validate no panic. |
| `cargo check` | Compile integration. |

### Batch 4: Error Diagnostics Cleanup

Scope:

| Fix | Files |
|---|---|
| P1-04 spawn failure logging | `load_pipeline.rs`, `gif_manager.rs`, `folder_preview_worker.rs`. |
| P2-03 MPV join timeout | `event_loop.rs`. |
| P2-04 mute `catch_unwind` cleanup | `playback_state.rs`, `media_preview.rs`. |

Verification:

| Command | Purpose |
|---|---|
| `cargo check` | Compile changed function signatures/logging. |

## 9. Final Notes For Fix Implementation

Do not add broad compatibility shims unless a concrete persisted or external behavior requires them.

Prefer small, targeted changes. The codebase is large and the highest-value fixes do not require broad architecture refactors.

When fixing UI-thread blocking in search tooltips, avoid moving blocking work into another UI callback. The desired invariant is: hover/render paths do not perform filesystem metadata reads, SQLite reads, image decode, or other disk I/O on cache miss.

When fixing GDI cleanup, be precise about handle ownership. `GetImage` handles must be deleted by the caller. `ISharedBitmap::GetSharedBitmap()` handles must not be deleted by the caller.
