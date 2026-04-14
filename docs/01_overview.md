# MTT File Manager — Overview

## Purpose

This document provides a high-level overview of MTT File Manager, its core capabilities, architecture, and technology stack.

## What is MTT File Manager

MTT File Manager is a native Windows file manager built in Rust with a modern borderless UI, tabbed navigation, integrated media preview, and deep Windows integration. It uses eframe/egui for rendering and communicates with a companion Windows Service for system-wide file search.

## Key Features

### Navigation & Interface
- **Custom borderless window** — No traditional title bar; native resize/move support
- **Dark / Light theme** — Toggle between dark and light mode in Settings > Appearance; setting is persisted in SQLite app state (`app_state.db`) and applied immediately across the main window, image viewer, and PDF viewer (including native Windows title bar via `DwmSetWindowAttribute`)
- **Tabbed navigation** — Multiple tabs with independent history per tab
- **Grid and List views** — Adjustable thumbnail sizes (64–512px)
- **Editable address bar** — Direct path input with breadcrumb navigation
- **Sidebar** — Quick access to drives, libraries, OneDrive, and Recycle Bin with auto-scroll on overflow
- **Quick Access (pinned folders)** — Pin folders via right-click or drag-and-drop; reorder via drag; persisted in SQLite app state (`app_state.db`)
- **Keyboard navigation** — Full keyboard shortcuts for mouse-free operation
- **Live search** — Type-to-filter files in the current folder
- **Internationalization** — English and Brazilian Portuguese via `rust-i18n`

### Preview & Media
- **Integrated preview panel** — View images, videos, GIFs, and PDFs without leaving the app
- **Dedicated image viewer** — Separate process with sliding-window cache, multi-threaded decoding, and instant navigation between images
- **Video player** — Standalone mpv-based player with D3D11 GPU pipeline, borderless window, and subtitle support
- **PDF viewer** — Native viewer using pdfium (Google's PDF rendering library via `pdfium-render` crate), with texture memory budgeting and page caching
- **Smart thumbnails** — Multi-stage generation pipeline: image crate → WIC → Shell API → force extract → Media Foundation
- **Custom folder covers** — Folder previews composed from 3 PNG layers (back/thumbnail/front) via `image` crate, replacing Shell API
- **Animated GIF playback** — Optimized GIF rendering with play/pause controls
- **Metadata extraction** — EXIF data from images, video/audio metadata via Media Foundation

### File Operations
- **Core operations** — Copy, cut, paste, rename, delete
- **Native context menu** — Full Windows Shell context menu integration
- **Recycle Bin** — Browse, restore, and permanently delete items
- **OneDrive support** — Sync status detection (cloud-only, syncing, pinned, locally available)
- **ISO mounting** — Mount ISO files as virtual drives
- **Inline renaming** — Rename files directly in the file list
- **Drag-and-drop** — Move/copy files via drag-and-drop

### Global Search
- **Dedicated overlay** — Activated via Ctrl+Shift+F
- **External service** — `mtt-search-service` communicating over Named Pipes (bincode serialization) with SQLite-backed volume snapshots and background FTS rebuilds
- **Hybrid volume indexing** — NTFS/ReFS via USN Journal; non-USN volumes (exFAT/FAT32/FUSE/CryptoFS) via full-tree scan
- **Adaptive update cadence** — USN incremental loop (2s); non-USN re-scan (30s for virtual filesystems, 120s for physical)
- **Paginated results** — Offset/limit pagination with incremental loading

### Cache & Performance
- **Split SQLite persistence** — Thumbnail cache, app state, directory metadata cache, and search-service index stored in dedicated databases
- **In-memory LRU cache** — Fast access via DashMap and LRU eviction
- **Async workers** — Background threads for thumbnails, icons, metadata, folder previews, file operations, and prefetch
- **Directory caching** — In-memory cache of directory structures for fast navigation
- **UI virtualization** — Only visible items are rendered in grid/list views
- **Adaptive batching** — Dynamic batch sizes for folder loading based on system performance
- **I/O prioritization** — Thread priority adjustment based on workload type

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Layer                             │
│  ┌─────────────┬──────────────┬───────────────────────┐    │
│  │   Toolbar   │   Tab Bar    │    Preview Panel      │    │
│  │   Sidebar   │   File List  │    Status Bar         │    │
│  └─────────────┴──────────────┴───────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                        │
│  ┌─────────────┬──────────────┬───────────────────────┐    │
│  │ Navigation  │ File Ops     │  Clipboard Manager    │    │
│  │ History     │ Sorting      │  Notification System  │    │
│  └─────────────┴──────────────┴───────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────┐
│                     Domain Layer                            │
│  ┌─────────────┬──────────────┬───────────────────────┐    │
│  │ FileEntry   │ Thumbnail    │  Error Types          │    │
│  │ Enums       │ FolderLock   │  PinnedFolder         │    │
│  └─────────────┴──────────────┴───────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────┐
│                  Infrastructure Layer                       │
│  ┌─────────────┬──────────────┬───────────────────────┐    │
│  │ Windows API │ Disk Cache   │  Media Foundation     │    │
│  │ Shell Integ.│ SQLite       │  Drive Watcher        │    │
│  └─────────────┴──────────────┴───────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

## Core Technologies

| Category | Technology | Version | Purpose |
|----------|-----------|---------|---------|
| Language | Rust | 2021 Edition | Core language |
| GUI Framework | eframe/egui | 0.31 | Immediate-mode GUI (features: `persistence`, `wgpu`) |
| Windows API | windows-rs | 0.61.0 | Native Windows integration |
| Database | SQLite (rusqlite) | 0.32 | Thumbnail cache, app state, directory metadata, and search persistence |
| Video | libmpv2 | 5.0.3 | Video playback |
| PDF | pdfium (pdfium-render) | 0.8.37 | Native PDF rendering |
| GPU Backend | wgpu (via eframe) | 24.0.5 | D3D12/Vulkan rendering with HighPerformance GPU preference |
| Images | image crate | 0.25 | Image processing (WebP, GIF) |
| SVG | resvg/usvg | 0.44 | SVG icon rendering |
| Parallelism | rayon | 1.10 | Parallel processing |
| Channels | crossbeam-channel | 0.5.15 | High-performance MPSC channels |
| Hashing | rustc-hash/fxhash | 2.0/0.2.1 | Fast hashing for PathBuf keys |
| Memory Mapping | memmap2 | 0.9 | Efficient large image reading |
| EXIF | kamadak-exif | 0.5 | JPEG metadata extraction |
| Compression | webp | 0.3 | WebP compression for thumbnails |
| Clipboard | clipboard-win | 5.4 | Windows clipboard (CF_HDROP) |
| File Dialogs | rfd | 0.15 | Native file dialogs |
| Watcher | Native Drive Watcher + notify (fallback) | native/6.1.1 | Filesystem monitoring (local + UNC) |
| i18n | rust-i18n | 3 | Multi-language support |
| IPC | Named Pipes + bincode | 1.3 | App ↔ search service communication |
| Windows Service | windows-service | 0.7 | Background indexing service |

## Runtime Dependencies

- **libmpv-2.dll** — Required for video playback (place in executable directory or PATH)
- **pdfium.dll** — Required for PDF viewer (place in executable directory or PATH)
- **Windows 10+** — Required for native Windows API integration

## Known Limitations

1. **Windows only** — Depends heavily on Windows APIs (Shell, COM, NTFS, WinRT)
2. **mpv dependency** — Video playback requires `libmpv-2.dll`
3. **Minimal test coverage** — Automated tests are sparse

## System Requirements

### Minimum
- Windows 10 (Build 1903+) or Windows 11
- x64 processor, 2+ cores
- 4 GB RAM
- 100 MB disk space + cache storage
- DirectX 12 or Vulkan capable GPU (via wgpu)

### Recommended
- Windows 11 (latest update)
- x64 processor, 4+ cores
- 8 GB RAM or more
- SSD for cache performance
- Dedicated GPU for video preview

## Further Reading

- [02_build_run_debug.md](02_build_run_debug.md) — Build, run, and debug instructions
- [03_architecture.md](03_architecture.md) — Detailed architecture
- [04_module_map.md](04_module_map.md) — Module map
- [05_dependencies_stack.md](05_dependencies_stack.md) — Full dependency stack
- [06_key_flows.md](06_key_flows.md) — Key application flows
- [07_storage_config.md](07_storage_config.md) — Storage and configuration
- [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) — Logging and error handling
- [09_performance_optimizations.md](09_performance_optimizations.md) — Performance optimizations

