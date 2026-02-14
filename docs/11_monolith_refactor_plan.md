# Monolith Refactor Implementation Plan

Last update: 2026-02-14

## Objective

Reduce god files/monoliths with incremental refactors, preserving behavior and keeping changes reviewable.

## Scope (Prioritized)

P0
- `src/app/operations/message_handler/thumbnail_events.rs`
- `src/app/operations/message_handler/watcher_events.rs`
- `src/app/init.rs`
- `src/infrastructure/disk_cache.rs`

P1
- `src/workers/file_operation_worker.rs`
- `src/ui/global_search_overlay.rs`
- `src/app/operations/folder_loading/load_pipeline.rs`
- `crates/mtt-search-service/src/main.rs`

P2
- `src/infrastructure/windows/codec_registry.rs`
- `src/infrastructure/windows/icons.rs`
- `src/ui/components/mpv_preview/mod.rs`

## Implementation Phases

## Phase 1 - Message Handler Decomposition (IN PROGRESS)

Goal
- Break large event-processing methods into focused modules/functions.

Targets
- Split thumbnail flow into: stream/rebuild, worker drains, upload policy, folder preview uploads.
- Split watcher flow into: event normalization, flood control, cache invalidation, reload policy.

Acceptance
- No behavioral changes.
- `cargo check` passes.
- Existing tests keep passing.

## Phase 2 - App Bootstrap Decomposition

Goal
- Replace monolithic `ImageViewerApp::new` with a small orchestrator.

Targets
- Create `src/app/init/` modules for channels, preferences, workers, layout restore, background jobs.

Acceptance
- Startup behavior identical.
- `init.rs` becomes thin composition layer.

## Phase 3 - Disk Cache Modularization

Goal
- Separate unrelated responsibilities currently in `disk_cache.rs`.

Targets
- Create `src/infrastructure/disk_cache/` with:
  - `schema.rs`
  - `thumbnails_repo.rs`
  - `preferences_repo.rs`
  - `folder_preview_repo.rs`
  - `folder_cover_repo.rs`
  - `gc.rs`

Acceptance
- Same DB schema and query behavior.
- GC/vacuum behavior preserved.

## Phase 4 - Worker and UI Monoliths

Targets
- `file_operation_worker.rs`: operation handlers by command.
- `global_search_overlay.rs`: input/debounce, filters, result list, activation.
- `load_pipeline.rs`: split pipeline stages and fallback branches.

## Phase 5 - Service and Windows Modules

Targets
- `mtt-search-service/src/main.rs`: service entry, discovery loop, indexer spawns, fallback loop.
- `codec_registry.rs`, `icons.rs`: break by concern.

## Delivery Strategy

- Small incremental PRs per file group.
- Baseline checks before/after each step:
  - `cargo check`
  - targeted tests when available
- No functional changes mixed with structure changes.

## Current Execution Log

2026-02-14
- Audit completed and priorities defined.
- Phase 1 started with first extraction in `thumbnail_events` (stream/rebuild logic).
- Thumbnail flow further decomposed into:
  - `thumbnail_workers.rs` (cover/icon/metadata/folder-size drains)
  - `thumbnail_uploads.rs` (thumbnail + folder preview upload pipeline)
- Watcher flow decomposition started with `watcher_reload.rs` (final auto-reload policy extraction).
- Watcher flow decomposed further into:
  - `watcher_drive_processing.rs` (flood handling + created/deleted/modified/renamed processing)
  - `watcher_legacy.rs` (notify-watcher fallback path)
- `init.rs` decomposition started with `init_preferences.rs`:
  - startup preferences loading/parsing moved out of `ImageViewerApp::new`.
- `init.rs` decomposition continued with `init_workers.rs`:
  - icon worker bootstrap extracted
  - metadata worker bootstrap extracted
  - disk cache invalidation worker bootstrap extracted
  - folder preview workers bootstrap extracted
  - folder size worker bootstrap extracted
  - prefetch/predictive/idle-warmup bootstraps extracted
  - file operation worker bootstrap extracted
  - global search worker bootstrap extracted
  - cover worker bootstrap extracted
  - async font loader bootstrap extracted
  - startup drive info preload job extracted
  - incremental GC background job extracted
- `init_workers` split to avoid a new monolith:
  - `src/app/init_workers/visual_workers.rs`
  - `src/app/init_workers/filesystem_workers.rs`
  - `src/app/init_workers/pipeline_workers.rs`
  - `src/app/init_workers/background_jobs.rs`
- `init.rs` decomposition continued with `init_state_builders.rs`:
  - `DriveState` assembly extracted
  - `LayoutState` assembly extracted
  - `FolderSizeState` assembly extracted
  - `FileOperationState` assembly extracted
- `init.rs` decomposition continued with `init_bootstrap.rs`:
  - cache + channel bootstrap extracted
  - worker/bootstrap wiring extracted
  - drive channel bootstrap extracted
- `init.rs` decomposition continued with `init_post_startup.rs`:
  - initial folder watch start extracted
  - startup drive-info preload trigger extracted
  - incremental GC trigger extracted
  - PDF warmup trigger extracted
- `disk_cache.rs` decomposition started with `disk_cache/gc.rs`:
  - incremental GC methods extracted
  - full GC + VACUUM methods extracted
  - GC path/drive helper methods extracted
- `disk_cache.rs` decomposition continued:
  - `disk_cache/preferences.rs` extracted
  - `disk_cache/folder_covers.rs` extracted
  - `disk_cache/folder_previews.rs` extracted
  - `disk_cache/cleanup.rs` extracted
  - `disk_cache/thumbnails_repo.rs` extracted
  - `disk_cache.rs` reduced to 310 lines
- `codec_registry.rs` decomposition started:
  - `codec_registry/known_codecs.rs` extracted
  - `codec_registry/mf_queries.rs` extracted
  - `codec_registry/registry_queries.rs` extracted
  - `codec_registry.rs` reduced to 443 lines
- `icons.rs` decomposition started:
  - `icons/file_icons.rs` extracted
  - `icons/thumbnails.rs` extracted
  - `icons/special.rs` extracted
  - `icons.rs` reduced to 15 lines
- `file_operation_worker.rs` decomposition started:
  - `file_operation_worker/handlers.rs` extracted
  - worker dispatch loop simplified (delegates by request type)
  - `file_operation_worker.rs` reduced to 277 lines
- `recycle_bin.rs` decomposition started:
  - `recycle_bin/enumeration.rs` extracted
  - `recycle_bin/operations.rs` extracted
  - `recycle_bin.rs` reduced to 117 lines
- `shell_operations.rs` decomposition started:
  - `shell_operations/context_menu.rs` extracted
  - `shell_operations/shfile_ops.rs` extracted
  - `shell_operations/file_op.rs` extracted
  - `shell_operations.rs` reduced to 12 lines
- `drive_watcher.rs` decomposition started:
  - `drive_watcher/buffer_parser.rs` extracted
  - `drive_watcher/thread_loop.rs` extracted
  - `drive_watcher.rs` reduced to 224 lines
- `workers/thumbnail/worker.rs` decomposition started:
  - `workers/thumbnail/worker/request_processing.rs` extracted
  - `worker.rs` reduced to 193 lines
  - targeted tests passed (`test_semaphore_concurrency`, `cache_entry_satisfies_request`)
- `security.rs` decomposition started:
  - `security/components.rs` extracted
  - `security/drive.rs` extracted
  - `security/symlink.rs` extracted
  - `security/unc.rs` extracted
  - `security.rs` now acts as API facade + tests
  - targeted tests passed (`security::tests::`, 14/14)
- `io_priority.rs` decomposition started:
  - `io_priority/detection.rs` extracted
  - `io_priority/threading.rs` extracted
  - `io_priority/grouped_queue.rs` extracted
  - `io_priority.rs` now acts as API facade + tests
  - targeted tests passed (`io_priority::tests::`, 3/3)
- `folder_loading/load_pipeline.rs` decomposition continued:
  - `load_pipeline/tier3_fallback.rs` extracted
  - OneDrive timeout fallback + Win32 FindFirstFileW path moved out of facade
  - `load_pipeline.rs` reduced to 121 lines
  - validation: `cargo check` passed
- `ui/global_search_overlay.rs` decomposition started:
  - `global_search_overlay/filters.rs` extracted
  - `global_search_overlay/results_panel.rs` extracted
  - `global_search_overlay.rs` reduced to 254 lines
  - targeted tests passed (`global_search_overlay::filters::tests::`, 3/3)
- `application/sorting.rs` decomposition started:
  - `sorting/sort_impl.rs` extracted
  - `sorting/filtering.rs` extracted
  - `sorting.rs` now acts as API facade + tests
  - targeted tests passed (`sorting::tests::`, 10/10)
- `ui/icon_loader.rs` decomposition started:
  - `icon_loader/async_ops.rs` extracted
  - `icon_loader/file_icons.rs` extracted
  - `icon_loader/special_icons.rs` extracted
  - `icon_loader.rs` reduced to facade + cache lifecycle methods
  - validation: `cargo check` passed
