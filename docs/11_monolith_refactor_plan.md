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
