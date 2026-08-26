# MTT File Manager

**Native Windows file manager** built in Rust with a modern UI, archive browsing, advanced media preview, and Windows integration.

<img width="3839" height="2062" alt="Untitled-1" src="https://github.com/user-attachments/assets/6ae29e6d-9f08-4d07-97bd-3e9e61a1f699" />

## Key Features

### Interface & Navigation
- **Dark / Light theme** — Toggle between dark and light mode in Settings > Appearance; persisted in SQLite, applied to all windows including image, PDF, and text viewers with native title bar support via DWM
- **Dual panel (split view)** — Side-by-side file browsing with independent left and right panels; toggle via the toolbar button. Each panel maintains its own navigation history, sort order, view mode, and selection. File copy/move operations default to the opposite panel as the destination
- **Tabbed navigation** — Multiple tabs with independent history
- **Grid, Details, Column List, and Miller's Columns views** — Switch between thumbnail, detailed table, horizontally scrolling list, and Finder-style hierarchical layouts. Miller's Columns keeps the folder hierarchy visible in side-by-side columns and supports keyboard navigation, multi-selection, rectangle selection, inline renaming, context menus, and drag-and-drop
- **File grouping** — Group items by name, date, type, or size in supported views; reverse the group order, collapse individual sections, and preserve grouping preferences between sessions
- **Per-folder view locks** — Preserve the view mode, sorting, and folder position for one folder or for that folder and all of its subfolders; inherited locks identify their source folder
- **Smart address bar** — Direct path input with breadcrumbs
- **Sidebar** — Quick access to drives, libraries, Cloud Drives, and Recycle Bin
- **Cloud Drives** — Detects Windows Cloud Files sync roots registered with Explorer and shows them in a dedicated sidebar section; tested with OneDrive, Proton Drive and Google Drive
- **Quick Access** — Pin folders via right-click or drag-and-drop; reorder via drag; persistent storage
- **Archive navigation** — Open supported compressed files like folders and browse their contents directly (`.zip`, `.7z`, `.rar`, `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`, `.tbz2`, `.tar.xz`, `.txz`, `.tar.zst`, `.tzst`, `.gz`, `.gzip`)

### Tags
- **Persistent color tags** — Assign custom color tags to files and folders from the context menu
- **Dedicated Tag views** — Browse every item assigned to a tag from the resizable Tags section in the sidebar
- **Tag management** — Create, rename, recolor, and delete tags from Settings, with usage counts shown before deletion
- **Global Search filters** — Show any tagged item or narrow search results to specific tags
- **File operation tracking** — Preserve tags across supported renames and moves, and clear them when files are deleted
- **Scalable browsing** — Load large Tag views in pages using cached file metadata while limiting thumbnail work to visible items

### Media Preview
- **Media metadata** — The preview panel extracts video and audio details such as codec, bitrate, channels, sample rate, and music tags
- **Smart thumbnails** — Multi-stage generation: image crate → WIC → Shell API → Media Foundation
- **Animated GIF playback** — Animated preview on details panel

### Image, PDF & Text Viewers
- **Consistent dedicated windows** — Images, PDF documents, and text files open in lightweight viewer windows with shared dark/light styling, themed toolbars, and native title bar integration
- **Image viewer** — Uses a bounded sliding-window GPU texture cache, hidden-first startup, multi-threaded decoding, zoom controls, and drag-to-pan navigation for zoomed images
- **PDF viewer** — Uses native PDFium rendering with asynchronous document loading, prioritized progressive rendering, virtualized pages and thumbnails, bounded texture caching, and keyboard navigation in the thumbnail sidebar
- **Text viewer** — Opens plain text, source code, logs, and markup files in a focused viewer with search and go-to-line controls

### Video Player
- **Standalone playback** — Opens video and audio files in an mpv-based player with a D3D11 GPU pipeline
- **Audio visualization** — Displays a real-time waveform while playing audio-only files
- **Playback controls** — Supports 5% volume steps with Up/Down and five-second seeking with the mouse wheel
- **Optical media playback** — Plays DVDs and Blu-rays from an optical drive's context menu
- **Optical media limitations** — DVD menus are not supported; protected DVDs depend on libdvdcss and physical-drive access, while AACS-protected Blu-rays require externally configured libaacs and keys

### Drive Information
- **System overview** — The This PC details panel shows the device name, CPU, installed RAM, GPU, Windows version, storage usage, motherboard, and BIOS information
- **Hardware details** — The details panel can show model, serial number, firmware, interface, standard, supported features, rotation, and current/maximum transfer modes for local drives
- **Health telemetry** — Reads standardized NVMe and ATA/SATA health data such as SMART status, temperature, remaining life, host reads/writes, power cycles, and power-on hours when the device protocol provides those values
- **NVMe link information** — Reports the current and maximum PCIe generation and link width when Windows exposes the corresponding PCI device properties
- **On-demand access** — Hardware queries run through the elevated `mtt-search-service` only when a local drive is shown in the details panel; successful results are cached for five minutes
- **Compatibility limits** — Native NVMe and standard ATA/SATA paths are prioritized. SAT-compatible USB bridges are supported on a best-effort basis, while proprietary USB bridges, RAID controllers, multi-disk volumes, and vendor-specific counter formats may not expose all fields

### Global Search
- **Instant search** — Query an in-memory index supporting millions of files, with trigram acceleration for eligible volumes and fallback matching for short queries and very large indexes
- **Hybrid volume indexing** — NTFS/ReFS via USN Journal; non-USN volumes via full-tree scan
- **Background service** — Dedicated Windows Service for continuous indexing
- **Spotlight-style overlay** — Activated by Ctrl+Shift+F
- **Paginated results** — Offset/limit pagination with incremental loading
- **Tag filters** — Narrow global search results to any tagged item or to specific tags
- **File interactions** — Select multiple results, use range selection, copy or cut files, rename inline, open the Windows context menu, and drag results to folders or other applications
- **Responsive tagged search** — Tagged results are resolved asynchronously and stale requests are cancelled when the query or filters change

> **Disclaimer:** Global Search reads the NTFS/ReFS USN Journal and MFT, and drive health queries access physical storage devices. Because Windows restricts these operations, the installer registers a dedicated Windows Service that runs with administrative rights. This is the **only** component of MTT File Manager that requires elevated installation privileges.

### File Operations
- **Core operations** — Copy, cut, paste, rename, delete
- **Batch rename** — Select 2+ files and press F2 to open the batch rename modal; configure a shared base name, number position (suffix/prefix), separator style (parentheses, underscore, dash, space, or none), and start/step/padding; drag-to-reorder; live preview table with per-row conflict detection
- **Native context menu** — Full Windows Shell context menu integration, including the native **New** submenu when right-clicking an empty folder area
- **Archive extraction** — Access the Windows Shell **Extract All** action directly from the context menu for supported archive files
- **External drag-and-drop** — Drag files from MTT File Manager to Windows Explorer and other compatible Windows applications
- **Tag assignment** — Add, remove, or switch file/folder tags from the context menu; tags are preserved on supported renames/moves and cleared when files are deleted
- **Recycle Bin** — Browse, restore, and permanently delete
- **Cloud Files support** — Sync status badges for cloud-only, locally available, syncing, and pinned files; supports Windows Cloud Files actions such as "Always keep on this device" and "Free up space"
- **ISO mounting** — Mount ISO files as virtual drives, detect images mounted before the app starts, and eject them from the sidebar

### Automatic File Organizer
- **Persistent rules** — Automatically move files from a source folder to a destination based on file extensions
- **Extension presets** — Quickly configure rules for documents, images, videos, audio, archives, and executables, or enter custom extensions
- **Safe folder monitoring** — Existing and newly created matching files are processed only after remaining stable for two seconds; destination conflicts are skipped and existing files are never overwritten
- **Protected moves and rule validation** — Cyclic rules are rejected, source files are checked before moving, and cross-drive copies are verified before their sources are removed
- **Preview and notifications** — Preview how many files match a rule before using it and receive batched details about completed moves, conflicts, and failures

### Performance & Cache
- **Multi-level cache** — Memory, disk (SQLite), and GPU textures
- **Tag view cache** — Persistent metadata cache and paged loading make large Tag views appear quickly without preloading thumbnails or increasing GPU texture cache limits
- **Async workers** — Background processing keeps UI responsive
- **UI virtualization** — Efficient rendering of large directories
- **Bounded background work** — Folder loading, thumbnail generation, and search rescans use concurrency limits to avoid resource spikes and improve shutdown behavior
- **Per-folder monitoring** — Default `notify` crate watcher with opt-in drive-wide `ReadDirectoryChangesW`; event bursts are coalesced into debounced reloads

## Graphics Backend

The app supports three rendering backend choices, selectable in **Settings > General > GPU Backend** (requires app restart):

### Wgpu — DirectX 12 (Default)
- Default backend for the main window on Windows
- Uses `wgpu` DX12 with a DirectComposition visual for presentation
- Avoids transient black frames when the borderless window is minimized
- If DX12 initialization fails, startup automatically retries Vulkan and then Glow + OpenGL

### Wgpu — Vulkan
- Optional Vulkan backend for the main window
- Keeps the Vulkan-specific thumbnail and grid performance tuning
- Does not use the DX12 DirectComposition presentation path

### Glow — OpenGL (Fallback)
- Recommended fallback when DirectX 12 is unavailable or unstable on the machine
- Uses eframe's `Glow` renderer directly instead of `wgpu`'s OpenGL backend
- OpenGL texture uploads can be synchronous on the CPU thread, so the app applies more conservative thumbnail and folder-preview upload limits on this backend

## Prerequisites

- **Windows 10 or newer, 64-bit** — The installer targets x64-compatible Windows systems.
- **Microsoft Visual C++ Redistributable 2015-2022 (x64)** — Required by the native runtime dependencies. The official Microsoft installer is available at: https://aka.ms/vs/17/release/vc_redist.x64.exe
- **Administrator permission during installation** — Required to install and start the Global Search and drive telemetry Windows Service (`mtt-search-service.exe`).
- **Video codecs for extended thumbnail support** — Optional, but recommended for formats not supported by Windows out of the box. See [Video Thumbnail Codecs](#video-thumbnail-codecs).

The main file manager does not need to run as administrator for normal file browsing and file operations. Elevated permission is isolated in `mtt-search-service.exe`, which indexes NTFS/ReFS volumes using low-level filesystem data and performs on-demand health queries against physical storage devices. The installer registers this dedicated Windows Service with the required privileges instead of requiring the whole application to run elevated.

## Technologies

| Category | Technology | Version | Purpose |
|----------|-----------|---------|---------|
| **Language** | Rust | Edition 2021 | Performance and safety |
| **GUI** | eframe/egui | 0.35 | Modern immediate-mode GUI (features: `persistence`, `wgpu`, `glow`) |
| **GPU Backend (Default)** | wgpu DirectX 12 via eframe | 29.x | DirectComposition presentation; requires app restart |
| **GPU Backend (Alternative)** | wgpu Vulkan via eframe | 29.x | Optional Vulkan renderer; requires app restart |
| **GPU Backend (Fallback)** | Glow OpenGL via eframe | via eframe | OpenGL fallback; requires app restart |
| **Windows API** | windows-rs | 0.62 | Native Windows integration |
| **Video** | libmpv2 | 5.0.3 | High-performance video playback |
| **PDF** | pdfium (pdfium-render) | 0.8.37 | Native PDF rendering (requires pdfium.dll) |
| **Database** | SQLite (rusqlite) | 0.32 | Reliable persistence |
| **Images** | image crate | 0.25 | Image processing |
| **Archives** | zip + sevenz-rust + tar + flate2/bzip2/xz2/zstd | 2 / 0.6 / 0.4 / 1 / 0.5 / 0.1 / 0.13 | Native archive handling for ZIP, 7z, TAR, and compressed TAR variants |
| **RAR** | unrar | 0.5 | Native RAR handling via the upstream UnRAR source |
| **Parallelism** | rayon | 1.10 | Parallel processing |
| **IPC** | Named Pipes + bincode | 1.3 | App ↔ search service communication |
| **Service** | windows-service | 0.7 | Background indexing and on-demand drive telemetry service |
| **i18n** | rust-i18n | 3 | Multi-language support (en, pt-BR) |

### Runtime Dependencies
- **libmpv-2.dll** — Required for video playback
- **pdfium.dll** — Required for PDF viewer
- **Video codecs** — Required for video thumbnail extraction (see [Video Thumbnail Codecs](#video-thumbnail-codecs) below)

## Diagnostic Mode Privacy Notes

- `Settings > Diagnostics` writes a privacy-filtered diagnostic file intended for technical troubleshooting with data minimization by design.
- The diagnostic file is meant to keep only technical information relevant to application behavior.
- File names, folder names, full paths, search text, and other sensitive or private user identifiers should not be exposed in this artifact. The log file is in plain text, so you can check it yourself to see all info collected and decied for yourself if you want to share or not.
- Nothing is sent automatically outside the application. The diagnostic file stays local unless the user chooses to share it.
- The feature auto-disables after 24 hours and keeps only the latest 10 MiB of filtered diagnostic events.
- This is a technical privacy measure for minimization and safer troubleshooting. It is not a standalone legal certification of LGPD or any other regulatory compliance.

## Video Thumbnail Codecs

The thumbnail pipeline uses 3 Windows APIs for video files: **Shell API** (Stage 3), **IThumbnailCache** (Stage 4), and **Media Foundation** (Stage 5). All three require video codecs to be registered on the system.

### What works out of the box (Windows 10/11)
- **MP4 (H.264/AVC)**, **WMV**, **AVI** — native Windows codecs

### What requires installation

| Format | Without codecs | With K-Lite Codec Pack |
|--------|---------------|------------------------|
| MP4 H.264 | ✅ Works | ✅ Works |
| MP4 HEVC/H.265 | ❌ Fails | ✅ Works |
| MKV (any codec) | ❌ Fails | ✅ Works |
| WEBM VP9/AV1 | ❌ Fails | ✅ Works |
| FLV | ❌ Fails | ✅ Works |

### Recommended: K-Lite Codec Pack

**[Download K-Lite Codec Pack (Standard)](https://codecguide.com/download_kl.htm)** — includes LAV Filters which register:
- **Thumbnail handlers** for Windows Shell (enables Stages 3 and 4)
- **Media Foundation decoders** (enables Stage 5)
- Support for **HEVC/H.265**, **VP9**, **AV1**, **MKV**, **WEBM**, **FLV**, and more

> **Note**: Without the appropriate codecs installed, all video thumbnail stages will fail silently and the file will display a generic icon instead.

## Credits

This project includes and builds upon work from the following projects:

- [ModernH](https://github.com/HarkeshBhatia/ModernH), by Harkesh Bhatia. Our OSC file originated from this project and is used here with small modifications.
- [RTX HDR / RTX VSR toggle gist](https://gist.github.com/anthonybaldwin/1e49b28b49babf64f159cb793c506333), by anthonybaldwin. This gist served as an early development reference while experimenting with RTX HDR / RTX VSR behavior in mpv; the current repository implementation has since been reworked independently.

## License

Except where otherwise noted, the original code and documentation authored for this repository are licensed under the **Apache License, Version 2.0**. See the top-level `LICENSE` and `NOTICE` files.

Apache-2.0 was chosen because it fits the current Rust stack well and gives a clear attribution baseline: anyone redistributing the Apache-licensed portions of this project must preserve the copyright notice, the license text, and any applicable `NOTICE` entries. In practice, this lets the project require retention of legal attribution, but it does **not** force public branding, UI credits, or endorsement for every downstream project.

This repository also contains or redistributes third-party components that remain under their own licenses and are not relicensed under Apache-2.0. Key examples include:

- `mpv_ui/portable_config/scripts/modernH.lua` and `mpv_ui/portable_config/script-opts/osc.conf`, derived from ModernH and kept under LGPL-2.1.
- `mpv_ui/portable_config/scripts/autoload.lua`, copied from upstream mpv tooling and governed by upstream mpv licensing.
- `target\release\libmpv-2.dll`, whose redistribution terms depend on the exact binary build shipped.
- `target\release\pdfium.dll`, which carries upstream PDFium licensing and notice obligations independent of the Rust bindings.
- `mpv_ui/portable_config/fonts/Material-Design-Iconic-Font.ttf`, which has its own upstream asset license.
- `unrar`, whose Rust wrapper is permissive but whose embedded UnRAR sources retain the upstream UnRAR license.

The official Windows installer is therefore a multi-license distribution, not
an Apache-only artifact. Public redistribution is intended to be allowed when
the installer keeps the bundled notices/license texts, the matching source code
or source locations remain available, and third-party components are not
described as being relicensed under Apache-2.0.

For practical release guidance, see `THIRD_PARTY_NOTICES.md` and the
`third_party_licenses/` bundle. Public installers include that directory, which
contains full license texts, attribution notes, source availability notes, and
release-sensitive binary provenance.
