# Key Flows — MTT File Manager

## 1. Folder Navigation

**Trigger**: User enters a path, clicks a folder, or navigates via breadcrumbs.

```
User Input
    ↓
navigate_to() [application/navigation.rs]
    ↓
load_folder() [app/operations/folder_loading/]
    ↓
read_directory_fast / read_directory_hdd_batched [infrastructure/ntfs_reader.rs, infrastructure/windows/hdd_directory_reader.rs]
    ↓
Sort & filter entries [application/sorting.rs]
    ↓
Update items in state [app/state/]
    ↓
Request thumbnails for visible items [app/operations/thumbnails.rs]
    ↓
Render file list [ui/views/grid_view/ or list_view/]
```

**Key files**: `application/navigation.rs`, `app/operations/folder_loading/`, `infrastructure/ntfs_reader.rs`, `infrastructure/windows/hdd_directory_reader.rs`, `application/sorting.rs`

## 2. File Preview

**Trigger**: User selects a file in the file list.

```
Selection event
    ↓
Check file type (image/video/PDF/GIF)
    ↓
┌──────────────┬──────────────────┬──────────────────┬──────────────┐
│ Image        │ Video            │ PDF              │ GIF          │
│ Decode via   │ mpv preview in   │ pdfium-render    │ Frame-based  │
│ image crate  │ embedded panel   │ (pdfium.dll)     │ animation    │
│ or WIC       │ (libmpv2)        │                  │              │
└──────────────┴──────────────────┴──────────────────┘──────────────┘
    ↓
Render in preview panel [ui/preview_panel/]
```

**Key files**: `ui/preview_panel/`, `ui/components/media_preview.rs`, `ui/components/gif_manager.rs`

## 3. File Operations (Copy/Move/Delete)

**Trigger**: Ctrl+C, Ctrl+X, Delete, or context menu action.

```
User action
    ↓
Clipboard or delete operation [app/operations/clipboard_ops.rs]
    ↓
Send to file operation worker [workers/file_operation_worker.rs]
    ↓
Execute via IFileOperation (Shell API) [infrastructure/windows/shell_operations.rs]
    ↓
Worker notifies completion via channel
    ↓
UI updates file list
```

**Key files**: `app/operations/clipboard_ops.rs`, `app/operations/file_ops.rs`, `workers/file_operation_worker.rs`, `infrastructure/windows/shell_operations.rs`

## 4. Thumbnail Generation (5-Stage Pipeline)

**Trigger**: Folder loads and visible items need thumbnails.

```
Request queued in PriorityThumbnailQueue
    ↓
Thumbnail worker picks item [workers/thumbnail/worker.rs]
    ↓
Stage 1: image crate (PNG, JPG, GIF, WebP)
    ↓ (fail?)
Stage 2: Windows Imaging Component (WIC)
    ↓ (fail?)
Stage 3: Shell API (IShellItemImageFactory)
    ↓ (fail?)
Stage 4: Force extraction
    ↓ (fail?)
Stage 5: Media Foundation (video files)
    ↓
Compress to WebP [webp crate]
    ↓
Store in SQLite disk cache [infrastructure/disk_cache/thumbnails_repo.rs]
    ↓
Send ThumbnailData via channel → UI loads as GPU texture
```

**Key files**: `workers/thumbnail/extraction/stage1_image_crate.rs` through `stage5_media_foundation.rs`, `workers/thumbnail/queue.rs`

## 5. Context Menu

**Trigger**: Right-click on a file or folder.

```
Right-click event
    ↓
Determine context (file, folder, background, Recycle Bin)
    ↓
Build custom menu items [app/operations/context_menu.rs]
    ↓
Option: show native Windows Shell menu [infrastructure/windows/native_menu.rs]
    ↓
Execute selected action
```

**Key files**: `ui/context_menu.rs`, `app/operations/context_menu.rs`, `infrastructure/windows/native_menu.rs`

## 6. Recycle Bin Operations

**Trigger**: Navigation to "Recycle Bin" or delete operations.

```
Navigate to Recycle Bin (special path)
    ↓
Enumerate deleted items [infrastructure/windows/recycle_bin.rs]
    ↓
Render in grid_view or list_view (with is_recycle_bin_view flag) with restore/delete options
    ↓
User action: Restore or Permanent Delete → Shell API
```

**Key files**: `app/operations/recycle_bin_ops.rs`, `infrastructure/windows/recycle_bin.rs`, `ui/views/grid_view/`, `ui/views/list_view/`

## 7. Keyboard Navigation

**Trigger**: Key press in file list.

| Key | Action |
|-----|--------|
| Arrow keys | Move selection |
| Enter | Open file/folder |
| Backspace | Navigate to parent |
| Delete | Move to Recycle Bin |
| Shift+Delete | Permanent delete |
| F2 | Rename |
| F5 | Reload folder |
| Ctrl+C / Ctrl+X | Copy / Cut |
| Ctrl+V | Paste |
| Ctrl+T | New tab |
| Ctrl+W | Close tab |
| Ctrl+Shift+F | Global search |
| Ctrl+L | Focus address bar |
| Type characters | Quick search filter |

**Key files**: `app/operations/navigation/keyboard.rs`, `ui/app/input.rs`

## 8. Quick Access (Pinned Folders)

**Trigger**: User pins/unpins a folder.

```
Pin action (right-click "Pin to Quick Access" or drag-and-drop to sidebar)
    ↓
Add to pinned_folders table in SQLite [infrastructure/disk_cache/pinned_folders.rs]
    ↓
Sidebar renders pinned folders section [ui/sidebar.rs]
    ↓
Reorder via drag-and-drop → update position in SQLite
    ↓
Unpin via 📌 icon → remove from SQLite
```

**Key files**: `app/operations/pinned_folder_ops.rs`, `infrastructure/disk_cache/pinned_folders.rs`, `ui/sidebar.rs`

## 9. Filesystem Monitoring

**Trigger**: App startup or folder navigation.

The app uses a layered filesystem monitoring strategy with the `notify` crate as the default watcher and the drive-wide `ReadDirectoryChangesW` watcher as an opt-in alternative.

### Default: Per-Folder Watcher (`notify` crate)

```
Navigate to folder [app/operations/watcher.rs]
    ↓
Create notify::RecommendedWatcher for current folder (NonRecursive)
    ↓
Events received via crossbeam channel (fs_event_receiver)
    ↓
process_legacy_notify_events() [app/operations/message_handler/watcher_legacy.rs]
    ↓
┌─────────────────┬──────────────────────────┐
│ DELETE event     │ ADD/MODIFY event         │
│ Remove from UI   │ Reload folder contents   │
│ (no full reload) │                          │
└─────────────────┴──────────────────────────┘
```

### Opt-In: Drive-Wide Watcher (ReadDirectoryChangesW)

**Activation**: Set `MTT_ENABLE_DRIVE_WATCHER=1` environment variable. Disabled by default because recursive `ReadDirectoryChangesW` on drive roots causes systemic UI degradation on machines with OneDrive/Cloud Files minifilters over prolonged use.

```
App init: start DriveWatcherManager (if MTT_ENABLE_DRIVE_WATCHER=1)
    ↓
Spawn one DriveWatcher per drive (ReadDirectoryChangesW on drive root)
    ↓
Async I/O with OVERLAPPED [infrastructure/drive_watcher/thread_loop.rs]
    ↓
Event received → parse buffer [infrastructure/drive_watcher/buffer_parser.rs]
    ↓
Filter events by current folder prefix
    ↓
On NTFS/ReFS: drop notify watcher entirely (drive watcher is sufficient)
On non-USN (exFAT/FAT): keep both watchers as resilience backup
```

### Resilience: Consistency Probe

A background worker (`app/init_workers/consistency_probe_worker.rs`) periodically computes a signature of the current directory listing and compares it against disk reality. This catches events that either watcher might miss (common on non-NTFS filesystems). Drives detected as unreliable are escalated to active polling mode.

### Special Case: User Session Search

The `user_session_search/` module uses `DriveWatcher` independently (not gated by `MTT_ENABLE_DRIVE_WATCHER`) to monitor virtual/FUSE volumes (e.g., Cryptomator/WinFsp mounts) for the in-app search index.

**Key files**: `app/operations/watcher.rs`, `app/operations/message_handler/watcher_legacy.rs`, `app/operations/message_handler/watcher_events.rs`, `infrastructure/drive_watcher.rs`, `infrastructure/drive_watcher_integration.rs`

## 10. Global Search

**Trigger**: Ctrl+Shift+F opens the search overlay.

```
User types query in overlay [ui/global_search_overlay/]
    ↓
Query sent to global_search_worker [workers/global_search_worker.rs]
    ↓
Worker connects via Named Pipe to mtt-search-service
    ↓
Service searches in-memory index (substring match, 3s deadline)
    ↓
Results returned: FRN → path_resolver → full paths
    ↓
Paginated results (offset/limit) rendered in overlay
    ↓
User clicks result → navigates to file's parent folder
```

**IPC Protocol**:
- Pipe: `\\.\pipe\MTTFileManagerSearch`
- Encoding: bincode with 4-byte length-prefix (LE)
- Fail-fast on `FILE_NOT_FOUND` (service not running)
- Retry only on `PIPE_BUSY` (service overloaded)

**Key files**: `ui/global_search_overlay/`, `workers/global_search_worker.rs`, `infrastructure/global_search.rs`

## 11. Folder Cover Composition

**Trigger**: Folder is visible and needs a cover image.

```
Folder visible in grid view
    ↓
Check folder_previews cache in SQLite [infrastructure/disk_cache/folder_previews.rs]
    ↓
Cache miss or stale? → Send to cover_worker [workers/folder_preview_worker.rs]
    ↓
Find first image/video in folder
    ↓
Compose 3-layer PNG via image crate [infrastructure/folder_compose.rs]:
  1. folder_back_512.png (folder silhouette — embedded via include_bytes!)
  2. Thumbnail of content (extracted by thumbnail pipeline)
  3. folder_front_512.png (folder tab overlay — embedded via include_bytes!)
    ↓
Store composed image in SQLite → send to UI
```

**Performance**: ~1-2ms per composition (embedded PNGs decoded once at startup).

**Key files**: `infrastructure/folder_compose.rs`, `workers/folder_preview_worker.rs`, `infrastructure/disk_cache/folder_previews.rs`

## 12. Dedicated Image Viewer

**Trigger**: Double-click on an image file.

```
Double-click image
    ↓
Spawn separate process: mtt-file-manager.exe --image-viewer <path>
    ↓
build_sequence(): read directory, filter images, natural sort [image_viewer/indexer.rs]
    ↓
Load first image synchronously (no spinner on open) [image_viewer/loader.rs]
    ↓
Start PrefetchEngine with worker pool [image_viewer/cache.rs]
    ↓
Sliding-window cache (radius=6, up to 13 images, 512MB budget)
    ↓
Navigate: Left/Right arrows
    ↓
Load new edge image only (tail-only fetch)
    ↓
Workers check AtomicUsize center before decoding (skip obsolete jobs)
    ↓
Previous image stays visible until new one is ready
```

**Key files**: `image_viewer/mod.rs`, `image_viewer/app/` (mod, filmstrip, rendering, gif_export), `image_viewer/cache.rs`, `image_viewer/indexer.rs`, `image_viewer/loader.rs`

## 13. Video Player

**Trigger**: Double-click on a video file.

```
Double-click video
    ↓
Spawn separate process: mtt-file-manager.exe --video-player <path> [--position <s>] [--volume <v>]
    ↓
Initialize mpv with D3D11 GPU pipeline [video_player/mod.rs]
    ↓
Borderless window with OSC controls
    ↓
Event loop: Shutdown, FileLoaded, EndFile, ClientMessage
    ↓
Subtitle loading via rfd file dialog
```

**Key files**: `video_player/mod.rs`

## 14. PDF Viewer

**Trigger**: Double-click on a PDF file.

```
Double-click PDF
    ↓
Spawn separate process: mtt-file-manager.exe --pdf-viewer <path>
    ↓
Path validation (no UNC, no traversal, .pdf extension, ≤512MB) [pdf_viewer/mod.rs]
    ↓
Load pdfium.dll dynamically (search next to exe, then system-wide) [pdf_viewer/renderer.rs]
    ↓
Render pages via pdfium-render crate [pdf_viewer/renderer.rs]
    ↓
Texture cache with memory budget and LRU eviction [pdf_viewer/viewer_app.rs]
    ↓
Async render worker for background page rendering [pdf_viewer/render_worker.rs]
    ↓
Text selection support [pdf_viewer/selection.rs]
    ↓
Toolbar for navigation [pdf_viewer/toolbar.rs]
```

**Key files**: `pdf_viewer/mod.rs`, `pdf_viewer/viewer_app.rs`, `pdf_viewer/renderer.rs`, `pdf_viewer/render_worker.rs`, `pdf_viewer/selection.rs`

