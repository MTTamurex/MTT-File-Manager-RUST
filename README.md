# MTT File Manager

**Native Windows file manager** built in Rust with a modern UI, archive browsing, advanced media preview, and Windows integration.

<img width="3840" height="2060" alt="MTT File Manager" src="https://github.com/user-attachments/assets/0873c298-c248-4d8c-b9fa-9e3068f8a4eb" />

## Key Features

### Interface & Navigation
- **Dark / Light theme** — Toggle between dark and light mode in Settings > Appearance; persisted in SQLite, applied to all windows including image, PDF, and text viewers with native title bar support via DWM
- **Dual panel (split view)** — Side-by-side file browsing with independent left and right panels; toggle via the toolbar button. Each panel maintains its own navigation history, sort order, view mode, and selection. File copy/move operations default to the opposite panel as the destination
- **Tabbed navigation** — Multiple tabs with independent history
- **Grid and List views** — Adjustable thumbnail sizes
- **Smart address bar** — Direct path input with breadcrumbs
- **Sidebar** — Quick access to drives, libraries, Cloud Drives, and Recycle Bin
- **Cloud Drives** — Detects Windows Cloud Files sync roots registered with Explorer and shows them in a dedicated sidebar section; tested with OneDrive, Proton Drive and Google Drive
- **Quick Access** — Pin folders via right-click or drag-and-drop; reorder via drag; persistent storage
- **Archive navigation** — Open supported compressed files like folders and browse their contents directly (`.zip`, `.7z`, `.rar`, `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`, `.tbz2`, `.tar.xz`, `.txz`, `.tar.zst`, `.tzst`, `.gz`, `.gzip`)

### Media Preview
- **Integrated preview** — View files without leaving the app
- **Dedicated image viewer** — Separate process with a bounded sliding-window GPU texture cache, hidden-first startup, and multi-threaded decoding
- **Text viewer** — Separate process for plain text, code, logs, and markup files using the same lightweight viewer runtime as the image/PDF viewers
- **Video player** — Standalone mpv-based player with D3D11 GPU pipeline
- **Audio playback & metadata** — Audio-only files open in the standalone mpv player with real-time waveform visualization; the preview panel extracts codec, bitrate, channels, sample rate, and music tags
- **PDF viewer** — Native pdfium-based viewer with bounded texture caching and asynchronous rendering in a separate process
- **Smart thumbnails** — Multi-stage generation: image crate → WIC → Shell API → Media Foundation
- **Animated GIF playback** — Animated preview on details panel

### Global Search
- **Instant search** — Query an in-memory index supporting millions of files
- **Hybrid volume indexing** — NTFS/ReFS via USN Journal; non-USN volumes via full-tree scan
- **Background service** — Dedicated Windows Service for continuous indexing
- **Spotlight-style overlay** — Activated by Ctrl+Shift+F
- **Paginated results** — Offset/limit pagination with incremental loading

> **Disclaimer:** The Global Search feature reads the NTFS/ReFS USN Journal and MFT to build its index. Because accessing these system structures requires elevated privileges, the installer registers a dedicated Windows Service that runs with administrative rights. This is the **only** component of MTT File Manager that requires elevated installation privileges.

### File Operations
- **Core operations** — Copy, cut, paste, rename, delete
- **Batch rename** — Select 2+ files and press F2 to open the batch rename modal; configure a shared base name, number position (suffix/prefix), separator style (parentheses, underscore, dash, space, or none), and start/step/padding; drag-to-reorder; live preview table with per-row conflict detection
- **Native context menu** — Full Windows Shell context menu integration
- **Recycle Bin** — Browse, restore, and permanently delete
- **Cloud Files support** — Sync status badges for cloud-only, locally available, syncing, and pinned files; supports Windows Cloud Files actions such as "Always keep on this device" and "Free up space"
- **ISO mounting** — Mount ISO files as virtual drives

### Performance & Cache
- **Multi-level cache** — Memory, disk (SQLite), and GPU textures
- **Async workers** — Background processing keeps UI responsive
- **UI virtualization** — Efficient rendering of large directories
- **Per-folder monitoring** — Default `notify` crate watcher with opt-in drive-wide `ReadDirectoryChangesW`

## Graphics Backend

The app supports multiple rendering backends, selectable in **Settings > General > GPU Backend** (requires app restart):

### Wgpu — Vulkan (Default)
- Default backend for the main window on Windows
- Best observed scrolling and thumbnail-upload performance in large media folders
- Uses Vulkan through `wgpu` with aggressive thumbnail memory cleanup tuned for this backend
- `Auto` mode prioritizes Vulkan first, then falls back to OpenGL and DirectX 12 if Vulkan is unavailable

### Experimental Backends — OpenGL / DirectX 12
- Available for compatibility testing from **Settings > General > GPU Backend**
- OpenGL options are fallback/legacy paths and may show occasional stutter during grid scrolling because texture uploads can be synchronous on the CPU thread
- **Known limitation DX12**: because wgpu creates the swapchain with `FLIP_DISCARD`, a brief black frame may flash during the minimize animation on Windows. This is a documented behavior of the wgpu DX12 backend and does not affect functionality.
- DX12 and OpenGL are currently treated as experimental compared with the Vulkan default

## Prerequisites

- **Windows 10 or newer, 64-bit** — The installer targets x64-compatible Windows systems.
- **Microsoft Visual C++ Redistributable 2015-2022 (x64)** — Required by the native runtime dependencies. The official Microsoft installer is available at: https://aka.ms/vs/17/release/vc_redist.x64.exe
- **Administrator permission during installation** — Required to install and start the Global Search Windows Service (`mtt-search-service.exe`).
- **Video codecs for extended thumbnail support** — Optional, but recommended for formats not supported by Windows out of the box. See [Video Thumbnail Codecs](#video-thumbnail-codecs).

The main file manager does not need to run as administrator for normal file browsing and file operations. Elevated permission is needed for the search service because Global Search indexes NTFS/ReFS volumes using low-level Windows filesystem data such as the USN Journal and MFT. Access to those structures is restricted by Windows, so the installer registers a dedicated Windows Service with the required privileges instead of requiring the whole application to run elevated.

## Usage

### Keyboard Shortcuts
Some app-level shortcuts are configurable in Settings > Keyboard Shortcuts. Standard file and folder shortcuts remain fixed.

| Shortcut | Action |
|----------|--------|
| Ctrl+T | New tab |
| Ctrl+W | Close tab |
| Ctrl+Tab | Next tab |
| Ctrl+Shift+Tab | Previous tab |
| Ctrl+C / Ctrl+V | Copy / Paste |
| Ctrl+X | Cut |
| Delete | Move to Recycle Bin |
| Shift+Delete | Permanent delete |
| F2 | Rename (single file) / Batch Rename modal (2+ files selected) |
| F5 | Reload folder |
| Ctrl+Shift+F | Global search |
| Ctrl+L | Focus address bar |
| Ctrl+Shift+N | New folder |
| Ctrl+Mouse Wheel | Adjust thumbnail size |
| Alt+Enter | Properties |
| Space | Video Preview / Open file with internal viewer (Images,PDF,Text)|

## Technologies

| Category | Technology | Version | Purpose |
|----------|-----------|---------|---------|
| **Language** | Rust | Edition 2021 | Performance and safety |
| **GUI** | eframe/egui | 0.31 | Modern immediate-mode GUI (features: `persistence`, `wgpu`, `glow`) |
| **GPU Backend (Default)** | wgpu Vulkan via eframe | 24.x | Main window uses **Vulkan** by default for best thumbnail/grid performance; Auto falls back to OpenGL/DX12 if needed |
| **GPU Backend (Experimental)** | OpenGL / DirectX 12 via eframe | via eframe / 24.x | Compatibility backends available from Settings; requires app restart |
| **Windows API** | windows-rs | 0.61.0 | Native Windows integration |
| **Video** | libmpv2 | 5.0.3 | High-performance video playback |
| **PDF** | pdfium (pdfium-render) | 0.8.37 | Native PDF rendering (requires pdfium.dll) |
| **Database** | SQLite (rusqlite) | 0.32 | Reliable persistence |
| **Images** | image crate | 0.25 | Image processing |
| **Archives** | zip + sevenz-rust + tar + flate2/bzip2/xz2/zstd | 2 / 0.6 / 0.4 / 1 / 0.5 / 0.1 / 0.13 | Native archive handling for ZIP, 7z, TAR, and compressed TAR variants |
| **RAR** | unrar | 0.5 | Native RAR handling via the upstream UnRAR source |
| **Parallelism** | rayon | 1.10 | Parallel processing |
| **IPC** | Named Pipes + bincode | 1.3 | App ↔ search service communication |
| **Service** | windows-service | 0.7 | Background indexing service |
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
