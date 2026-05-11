# Architecture — MTT File Manager

## Workspace Structure

The project is organized as a Cargo Workspace with 3 crates:

```
MTT-File-Manager-RUST/
├── Cargo.toml                    # Workspace root + mtt-file-manager package
├── src/                          # Main app (GUI)
├── crates/
│   ├── mtt-search-protocol/     # Shared IPC types (SearchRequest, SearchResponse)
│   └── mtt-search-service/      # Windows Service for hybrid indexing + Named Pipe IPC
```

| Crate | Type | Description |
|-------|------|-------------|
| `mtt-file-manager` | bin (GUI) | Main application with eframe/egui; also hosts `--image-viewer`, `--pdf-viewer`, `--text-viewer`, `--video-player`, and `--set-volume-label` standalone entry points |
| `mtt-search-protocol` | lib | IPC types and bincode serialization |
| `mtt-search-service` | bin (service) | Windows Service with hybrid per-volume indexing, binary/SQLite startup caches, and in-memory SIMD search over Named Pipe IPC |

## Architecture Overview

The application follows a layered architecture with clear separation of responsibilities:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Presentation Layer                                │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                           UI Layer                                  │    │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │    │
│  │  │  Toolbar   │  Tab Bar   │ File List  │  Sidebar   │ Preview  │  │    │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │    │
│  │  ┌───────────────────────────────────────────────────────────────┐  │    │
│  │  │      Dual Panel (split left / right panels)                  │  │    │
│  │  └───────────────────────────────────────────────────────────────┘  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    eframe/egui Framework                             │    │
│  │                    (Immediate Mode GUI)                             │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Application Layer                                   │
│  ┌────────────┬────────────┬────────────┬────────────┬──────────────────┐  │
│  │ Navigation │ File Ops   │ Clipboard  │ Sorting    │ Watcher Service  │  │
│  │ History    │ Manager    │ Manager    │ Engine     │ & Notifications  │  │
│  └────────────┴────────────┴────────────┴────────────┴──────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Domain Layer                                      │
│  ┌────────────┬────────────┬────────────┬────────────┬──────────────────┐  │
│  │ FileEntry  │ Thumbnail  │ SortMode   │ ViewMode   │ Error Types      │  │
│  │ DriveInfo  │ Data       │ Enum       │ Enum       │ (AppError)       │  │
│  └────────────┴────────────┴────────────┴────────────┴──────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────────────────┐
│                       Infrastructure Layer                                  │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                    Windows Integration                              │    │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │    │
│  │  │ Shell API  │ Filesystem │ Media      │ Thumbnail  │ COM API  │  │    │
│  │  │ Integ.     │ Operations │ Foundation │ Extraction │ Wrapper  │  │    │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                     Data Layer                                      │    │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │    │
│  │  │ SQLite     │ Filesystem │ Memory     │ Directory  │ Config   │  │    │
│  │  │ Cache      │ Access     │ Cache      │ Index      │ Storage  │  │    │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────────────────────┐    │
│  │                   Worker Threads                                    │    │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │    │
│  │  │ Thumbnail  │ File Ops   │ Prefetch   │ Folder     │ Icon     │  │    │
│  │  │ Workers    │ Worker     │ Worker     │ Preview    │ Worker   │  │    │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │    │
│  │  ┌────────────────────────────────────────────────────────────────┐ │    │
│  │  │Global Search Worker (Named Pipe client → mtt-search-service)  │ │    │
│  │  └────────────────────────────────────────────────────────────────┘ │    │
│  └─────────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────────────────┐
│                  External: Search Service (separate process)                │
│  ┌────────────┬────────────┬────────────┬────────────┬──────────────────┐  │
│  │ USN/FS     │ File Index │ Path       │ SQLite     │ Named Pipe IPC   │  │
│  │ Scan       │ (HashMap)  │ Resolver   │ Persist.   │ Server           │  │
│  └────────────┴────────────┴────────────┴────────────┴──────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                                │
┌─────────────────────────────────────────────────────────────────────────────┐
│   External: Standalone Viewers (image / PDF / text, separate process)      │
│  ┌────────────┬────────────┬────────────┬────────────┬──────────────────┐  │
│  │ Viewer     │ Shared     │ Bounded    │ Per-format │ Optional IPC /   │  │
│  │ Window     │ Runtime    │ Caches     │ Loader /   │ Worker Threads   │  │
│  │ (eframe)   │ (Glow)     │            │ Renderer   │                  │  │
│  └────────────┴────────────┴────────────┴────────────┴──────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Layers & Responsibilities

### 1. Presentation Layer (UI)
**Location**: `src/ui/`

Renders the user interface using eframe/egui (immediate-mode GUI).

**Components**:
- `src/ui/toolbar.rs` — Top toolbar with action buttons
- `src/ui/tab_bar/` — Tab system (renderer, controls, drag-dwell)
- `src/ui/views/` — File views (grid_view, list_view, computer_view)
- `src/ui/sidebar.rs` — Side panel with drives and shortcuts
- `src/ui/sidebar_tree.rs` — Tree sidebar for folder navigation
- `src/ui/preview_panel/` — File preview panel with video support
- `src/ui/status_bar.rs` — Bottom status bar
- `src/ui/app/` — App lifecycle, input handling, notifications, panels
- `src/ui/app_impl.rs` — Main `eframe::App` implementation
- `src/ui/app/panels/content.rs` — Dual panel layout rendering (header, left/right content areas, focus switching)
- `src/ui/components/` — Reusable widgets (media_preview, gif_manager, item_slot, mpv, mpv_preview, language_settings, appearance_settings, video_controls_state, virtual_drive_settings)
- `src/ui/global_search_overlay/` — Global search overlay UI
- `src/ui/icon_loader/` — Icon extraction and loading
- `src/ui/cache.rs` — Texture/icon cache manager (CacheManager)
- `src/ui/theme.rs` — UI theming
- `src/ui/widgets.rs` — Custom egui widgets
- `src/ui/svg_icons.rs` — SVG icon renderer
- `src/ui/navigation.rs` — Navigation UI
- `src/ui/context_menu.rs` — Context menu rendering

### 2. Application Layer
**Location**: `src/application/`

Business logic and application services.

- `navigation.rs` — Navigation history management
- `file_operations.rs` — File copy/move/delete operations
- `clipboard.rs` — Clipboard management
- `sorting.rs` — Sorting facade (`sort_items`, `filter_items`)
- `sorting/sort_impl.rs` — Sort implementation
- `sorting/filtering.rs` — Filter implementation
- `watcher.rs` — Filesystem change monitoring integration
- `notification.rs` — Toast notification system
- `renaming.rs` — File rename logic
- `context_menu.rs` — Context menu logic

**Dual Panel** (`src/app/dual_panel.rs`, `src/app/operations/dual_panel_ops.rs`):

The dual panel subsystem allows side-by-side browsing of two independent directory locations. The active panel occupies the main `ImageViewerApp` fields; the inactive panel's complete state (navigation, items, selection, scroll, view preferences, per-panel generation counter) is captured in a `PanelSnapshot`. Switching panels performs a zero-allocation field swap via `swap_with_app`. Each panel has an independent `generation: usize` and a private `Arc<AtomicUsize>` for routing async folder-load results so workers targeting the inactive panel are never cancelled by the active panel's generation advances. The shared `current_generation` Arc is restored to the active panel's generation on every `with_inactive_panel` exit so thumbnail workers always accept requests from the correct panel.

### 3. Domain Layer
**Location**: `src/domain/`

Core data models and business rules.

- **`file_entry.rs`** — `FileEntry`, `DriveInfo`, `SortMode`, `ViewMode`, `FoldersPosition`, `SyncStatus`, `IconSize`
- **`thumbnail.rs`** — `ThumbnailData` struct
- **`errors.rs`** — `AppError` enum with variants: Security, WindowsApi, Io, ThumbnailExtraction, FileOperation, InvalidState, Config, Worker, UiRendering
- **`folder_lock.rs`** — `FolderLock` struct (per-folder view preferences)
- **`pinned_folder.rs`** — `PinnedFolder` struct (Quick Access items)
- **`special_paths.rs`** — System paths (Computer view, Recycle Bin)

### 4. Infrastructure Layer
**Location**: `src/infrastructure/`

System access, Windows integration, and data persistence.

**Cache & Storage**:
- `db_utils.rs` — Shared SQLite helpers (ACL hardening, temp fallback, PRAGMA setup)
- `disk_cache.rs` + `disk_cache/` — SQLite-backed thumbnail cache (`thumbnails`, `folder_previews`, `shell_icons`)
- `app_state_db/` — SQLite-backed app state (`user_preferences`, `folder_locks`, `pinned_folders`, `folder_covers`)
- `directory_cache.rs` — In-memory directory cache
- `directory_index.rs` — Persisted directory metadata cache (`cache/directory_cache.db`)
- `icon_disk_cache.rs` — Icon disk cache layer
- `adaptive_batch.rs` — Adaptive batch configuration for folder loading

**Filesystem**:
- `ntfs_reader.rs` — NTFS raw directory reading (`NtQueryDirectoryFile`)
- `drive_watcher.rs` + `drive_watcher/` — Low-level `ReadDirectoryChangesW` watcher (buffer_parser, thread_loop)
- `folder_compose.rs` — Custom folder cover composition (3-layer PNG)
- `virtual_drive_config.rs` — Virtual drive and disk type configuration
- `io_priority.rs` + `io_priority/` — I/O priority management (detection, grouped_queue, threading)
- `directory_dirty_registry.rs` — Directory dirty state tracking

**Windows Integration** (`src/infrastructure/windows/`):
- `shell_operations.rs` + `shell_operations/` — File operations via Shell API (`IFileOperation`)
- `icons.rs` + `icons/` — Windows icon extraction
- `recycle_bin.rs` + `recycle_bin/` — Recycle Bin operations
- `native_menu.rs` — Native Windows context menu
- `media_foundation.rs` — Media Foundation for video thumbnails
- `metadata/` — Image, video, and audio metadata extraction
- `drives.rs` — Drive enumeration
- `file_system.rs` — Filesystem operations
- `file_type.rs` — File type detection
- `file_flags.rs` — Windows file flags
- `folder_size.rs` — Folder size calculation
- `formatting.rs` — String/number formatting
- `hdd_directory_reader.rs` — Optimized HDD directory reader
- `iso_mount.rs` — ISO mounting
- `bitmap_conversion.rs` — Windows bitmap conversion
- `codec_registry.rs` + `codec_registry/` — Media codec name cache
- `device_change.rs` — Device change monitoring
- `shell_folder.rs` — Shell special folders
- `system_info.rs` — System information
- `window_corners.rs` — Window corner styling and native dark title bar (`DwmSetWindowAttribute` with `DWMWA_USE_IMMERSIVE_DARK_MODE`)
- `window_subclass.rs` — Window subclassing for customization

**Other Infrastructure**:
- `global_search.rs` — Named Pipe client for search service IPC (search, status, warm-up, folder-size fast path, path freshness checks)
- `shell_menu_worker.rs` — Shell context menu extraction worker
- `user_session_search/` — User session search index (split module: orchestration, db persistence, discovery, scanner)
- `security.rs` + `security/` — Security validation (components, drive, shell_namespace, symlink, unc)
- `windows_clipboard.rs` — Windows clipboard (`CF_HDROP`)
- `viewer_runtime.rs` — Shared low-baseline runtime helpers for image/PDF/text viewer subprocesses (read-only prefs, Glow renderer config)
- `onedrive/` — OneDrive integration (path_detection, attributes, timeout_ops, directory_enum, pin_state)
- `media/` — Media infrastructure (hardware_acceleration)
- `archive_extract.rs` — Native archive extraction fallback (ZIP, 7z, RAR, TAR variants)
- `threading.rs` — Named thread spawning utilities

### 5. Workers Layer
**Location**: `src/workers/`

Background threads for asynchronous processing.

- `thumbnail/` — Image-aware multi-stage thumbnail system
    - `extraction/stage0_embedded_exif_thumbnail.rs` — JPEG embedded EXIF thumbnail fast path
    - `extraction/stage1_image_crate.rs` — Image crate fallback for common formats
    - `extraction/stage2_wic.rs` — Windows Imaging Component image-target path and robust fallback
    - `extraction/stage3_shell_api.rs` — Shell API (`IShellItemImageFactory`) fallback and video handoff
    - `extraction/stage4_force_extract.rs` — Forced extraction
    - `extraction/stage5_media_foundation.rs` — Media Foundation (videos)
  - `queue.rs`, `types.rs`, `worker.rs`, `processing/` — Queue, types, worker loop, and post-processing
- `folder_preview_worker.rs` — Folder cover composition worker
- `file_operation_worker.rs` + `file_operation_worker/` — Async file operations
- `prefetch_worker.rs` — Directory prefetching
- `idle_warmup.rs` — Idle-time cache warmup
- `global_search_worker.rs` — Global search IPC worker with query coalescing

### 6. Search Service (External Process)
**Location**: `crates/mtt-search-service/`

Separate Windows Service that indexes all files with a hybrid per-volume strategy, persists restart snapshots under `C:\ProgramData\MTT-File-Manager` in SQLite plus per-volume binary caches, and serves searches via Named Pipes from an in-memory SIMD matcher. Runs as `LocalSystem`.

**Modules**:
- `usn_journal.rs` — Volume discovery (`discover_volumes`) and USN API (NTFS/ReFS)
- `fs_walker.rs` — Full-tree scanner for non-USN volumes
- `file_index.rs` — In-memory index: `HashMap<u64, FileRecord>` (FRN → record)
- `path_resolver.rs` — Full path reconstruction via parent FRN chain
- `index_db/` — Persistence layer (shared data dir, SQLite schema/metadata, binary snapshot save/load with CRC, record sync, integrity verification)
- `ipc_server/` — Named Pipe server (split module: server loop, pipe I/O with DACL security, request handler)
- `ipc_authorization.rs` — IPC authorization handling
- `security_policy.rs` — Security policy configuration
- `service_control.rs` — Service install/uninstall via `windows-service`
- `name_arena.rs` — String arena for name storage
- `volume_indexers/` — Per-volume indexer management (split module: orchestration, USN journal indexer, non-USN full-tree indexer)

**IPC Protocol** (`crates/mtt-search-protocol/`):
- Serialization via **bincode** with 4-byte length-prefix framing (LE)
- Pipe: `\\.\pipe\MTTFileManagerSearch`
- Requests: `Query`, `GetStatus`, `Ping`, `WarmIndex`, `CheckPathsModified`, `FolderSize`
- Responses: `Results`, `Status`, `Pong`, `WarmStarted`, `PathsModified`, `FolderSize`, `Error`

**Indexing flow**:
1. Detect mounted volumes via `GetVolumeInformationW` and mark `usn_supported` for NTFS/ReFS
2. Spawn 1 indexer thread per discovered volume
3. USN volumes (NTFS/ReFS): try per-volume binary snapshot `index_<drive>.bin` first, fall back to SQLite rows in `search_index.db`, validate `journal_id`, then catch up via USN; if caches are missing or stale, run `read_mft_bulk()` over raw `$MFT` data runs to extract names, sizes, parents, hardlinks, and reparse metadata in one sequential pass; write a fresh binary snapshot, keep SQLite volume metadata in sync, and enter the 2 s incremental loop
4. Legacy USN caches that do not contain sizes can mark the volume `Ready` quickly, then finish size extraction in a background bulk-MFT pass while `sizes_loading` remains visible to the client
5. Non-USN volumes: optionally reuse cached SQLite rows for fast startup, then run `fs_walker` full scans, persist `file_records`/`hardlink_parents` back to SQLite, and rescan periodically (30 s on virtual filesystems, 120 s on physical filesystems) with change notifications where supported
6. Search requests run against the live in-memory `VolumeIndex` using the lowered `NameArena`; binary/SQLite persistence accelerates startup and recovery but is not on the hot query path
7. Discovery loop runs every 20 s to detect newly mounted volumes

### 7. Image Viewer (Separate Process)
**Location**: `src/image_viewer/`

Dedicated image viewer running as a **separate process** (same binary, `--image-viewer` flag). Memory and GPU textures are released by the OS when the process closes.

**Modules**:
- `mod.rs` — `open_image_viewer()` offloads validation/spawn to a background thread, forwards to an existing instance via IPC when possible, and `run_standalone()` initializes the viewer window
- `app/` — `DedicatedImageViewerApp` (split module: struct & navigation in `mod.rs`, filmstrip in `filmstrip.rs`, UI rendering in `rendering.rs`, GIF/export in `gif_export.rs`)
- `cache.rs` — `WindowCache` (GPU `TextureHandle` sliding-window cache) + `PrefetchEngine` (crossbeam bounded channel workers, atomic center tracking)
- `indexer.rs` — `build_sequence()`: reads directory, filters images, natural sort
- `loader.rs` — Decoding: memory-mapped files for >1MB, EXIF orientation, WIC fallback
- `metrics.rs` — Performance metrics
- `ipc.rs` — Inter-process communication

**Cache architecture**:
- Sliding-window with radius=1 (current image + immediate neighbors)
- Cache stores GPU `TextureHandle`s instead of CPU-side RGBA buffers
- Workers check `AtomicUsize` center before decoding — obsolete jobs are skipped
- Bounded channels sized at `2*radius+1` to prevent infinite job accumulation
- Navigation requests only the new edge image (tail-only), not the full window
- Previous image remains visible until the new one is ready (no spinner during fast navigation)
- Startup does not seed from the file-manager thumbnail cache; the viewer decodes its own full-frame image path
- The viewport starts hidden (`with_visible(false)`) and is revealed on the first viewer frame

### 8. Video Player (Separate Process)
**Location**: `src/video_player/`

Standalone mpv-based video player launched as a separate process (`--video-player <path> [--position <s>] [--volume <v>]`).

- Borderless native mpv window with custom OSC controls
- D3D11 GPU pipeline (`vo=gpu-next`, `gpu-api=d3d11`, `hwdec=d3d11va`)
- Subtitle picker via native file dialog (rfd)
- Playback position and volume passed via CLI args
- Event loop handles `Shutdown`, `FileLoaded`, `EndFile`, `ClientMessage`

### 9. PDF Viewer (Separate Process)
**Location**: `src/pdf_viewer/`

Native PDF viewer using **pdfium** (Google's PDF rendering library via `pdfium-render` crate). Requires `pdfium.dll` at runtime. Launched as a separate process (`--pdf-viewer` flag).

- Dynamic loading of `pdfium.dll` (searches next to executable, then system-wide)
- Path validation: blocks UNC paths, null bytes, path traversal, and non-`.pdf` extensions
- File size limit: 512 MB
- Texture cache bounded by `TEXTURE_MEMORY_BUDGET = 128 MB`
- Render worker keeps a persistent Pdfium document handle and uses bounded channels for asynchronous page rendering
- Text selection support
- Toolbar for navigation controls

### 10. Text Viewer (Separate Process)
**Location**: `src/text_viewer/`

Native text viewer launched as a separate process from the same executable (`--text-viewer` flag).

- Path validation: blocks UNC paths, null bytes, path traversal, and non-text extensions
- File size limit: 25 MB
- Stores text as `content: String + line_offsets: Vec<u32>` instead of `Vec<String>` to reduce memory overhead on large files
- Uses the shared `viewer_runtime.rs` lightweight setup (read-only prefs + `Glow` renderer)

## Key Boundaries

### UI ↔ Application
- Communication via MPSC channels (`crossbeam-channel`)
- State shared via `Arc<>` and channels

### Application ↔ Infrastructure
- Error conversion via `thiserror` and `AppError`
- Worker threads for I/O operations

### Windows Integration
- `windows-rs` crate for safe bindings
- COM initialization and resource management via RAII

### App ↔ Image Viewer
- Separate process via `Command::new(exe).arg("--image-viewer").arg(path)`
- Full memory isolation; OS reclaims everything on close

### App ↔ PDF/Text Viewers
- Separate process via `Command::new(exe).arg("--pdf-viewer").arg(path)` / `Command::new(exe).arg("--text-viewer").arg(path)`
- Shared lightweight `viewer_runtime.rs` keeps startup and baseline memory lower than the main window

## Application Lifecycle

```
main.rs
    ↓
eframe::run_native()
    ↓ (creation callback)
ImageViewerApp::new() [app/init.rs]
    ↓
ImageViewerApp::update() [ui/app_impl.rs] ←──┐
    ↓                                          │
Process Input → Update State → Render UI       │ (60 FPS loop)
    ↑                                          │
    └──────────────────────────────────────────┘
```

### Startup (main.rs → app/init.rs)
1. Initialize codec registry
2. Load app icon, configure viewport (borderless, hidden initially)
3. Read `gpu_backend` preference from `app_state.db` before eframe init
4. Call `eframe::run_native()` with `Wgpu` renderer and `HighPerformance` preference
5. In `ImageViewerApp::new()`:
   - Create communication channels (multiple workers)
   - Initialize worker threads (thumbnails, files, icons, metadata, covers, folder previews)
   - Load preferences from `app_state.db` (including theme mode)
   - Apply saved theme visuals (`Visuals::dark()` / `Visuals::light()`) on first frame
   - Configure caches and indices
   - Initialize filesystem watcher (`notify` per-folder watcher + consistency probe)
   - Load initial state
   - Configure custom fonts (async font loading)

### Main Loop (ui/app_impl.rs)
1. Process worker messages (thumbnails, files, icons, metadata)
2. Process filesystem events (watcher)
3. Update UI state
4. Process user input (keyboard, mouse)
5. Render components
6. Update cache and thumbnails
7. Manage animations (GIFs, videos)

### Shutdown
- Workers finalize when channels are dropped
- Cache is persisted automatically
- COM resources released via RAII
- Force-kill mechanism via `cancel_pending_io_on_current_process_threads` and `terminate_current_process` to prevent zombie hangs

## Communication Patterns

### MPSC Channels
```rust
// UI → Worker (send work)
worker_sender.send(work_item);

// Worker → UI (send result)
ui_sender.send(result);

// UI receives in update loop
while let Ok(result) = receiver.try_recv() {
    // Update state
}
```

### Active Worker Channels
| Worker | Sender | Receiver | Data Type |
|--------|--------|----------|-----------|
| Thumbnail | `thumbnail_queue` | `image_receiver` | `ThumbnailData` |
| File Entry | — | `file_entry_receiver` | `(generation, Vec<FileEntry>)` |
| Icon | `icon_req_sender` | `icon_res_receiver` | `(PathBuf, Vec<u8>, u32, u32)` |
| Metadata | `metadata_req_sender` | `metadata_res_receiver` | `(PathBuf, u64, MediaMetadata)` |
| Cover | `cover_worker_sender` | `cover_worker_receiver` | `(PathBuf, Option<PathBuf>)` |
| Folder Preview | `folder_preview_sender` | `folder_preview_receiver` | `FolderPreviewData` |
| Global Search | — | `global_search_receiver` | `GlobalSearchResponse` |

## Extension Points

### New Preview Types
1. Implement in `src/ui/preview_panel/`
2. Add component in `src/ui/components/`
3. Register in `src/app/operations/view_setup.rs`

### New File Operations
1. Add to `src/application/file_operations.rs`
2. Implement handler in `src/app/operations/file_ops.rs`
3. Add UI in toolbar/context menu

### New Windows Integrations
1. Add module in `src/infrastructure/windows/`
2. Export in `src/infrastructure/windows/mod.rs`
3. Use `AppError` for error handling

### New Workers
1. Create in `src/workers/`
2. Add channels to `ImageViewerApp` state
3. Initialize in `app/init.rs`
4. Process messages in `ui/app_impl.rs`
