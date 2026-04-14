# MTT File Manager

**Native Windows file manager** built in Rust with a modern UI, advanced media preview, and deep Windows integration.

## About

MTT File Manager is a desktop file manager that combines Rust's performance and safety with a modern interface and native Windows integration. It offers tabbed navigation, integrated file preview, and advanced management features.

## Key Features

### Interface & Navigation
- **Custom borderless window** — Modern frameless UI with native resize support
- **Dark / Light theme** — Toggle between dark and light mode in Settings > Appearance; persisted in SQLite, applied to all windows including image and PDF viewers with native title bar support via DWM
- **Tabbed navigation** — Multiple tabs with independent history
- **Grid and List views** — Adjustable thumbnail sizes
- **Smart address bar** — Direct path input with breadcrumbs
- **Sidebar** — Quick access to drives, libraries, OneDrive, and Recycle Bin
- **Quick Access** — Pin folders via right-click or drag-and-drop; reorder via drag; persistent storage

### Media Preview
- **Integrated preview** — View files without leaving the app
- **Dedicated image viewer** — Separate process with sliding-window cache, instant navigation, and multi-threaded decoding
- **Video player** — Standalone mpv-based player with D3D11 GPU pipeline
- **Audio playback & metadata** — Audio-only files open in the standalone mpv player with real-time waveform visualization; the preview panel extracts codec, bitrate, channels, sample rate, and music tags
- **PDF viewer** — Native viewer using pdfium (Google's PDF rendering library via `pdfium-render` crate)
- **Smart thumbnails** — Multi-stage generation: image crate → WIC → Shell API → Media Foundation
- **Animated GIF playback** — Optimized rendering with play/pause controls

### Global Search
- **Instant search** — Query an in-memory index supporting millions of files
- **Hybrid volume indexing** — NTFS/ReFS via USN Journal; non-USN volumes via full-tree scan
- **Background service** — Dedicated Windows Service for continuous indexing
- **Spotlight-style overlay** — Activated by Ctrl+Shift+F
- **Paginated results** — Offset/limit pagination with incremental loading

### File Operations
- **Core operations** — Copy, cut, paste, rename, delete
- **Native context menu** — Full Windows Shell context menu integration
- **Recycle Bin** — Browse, restore, and permanently delete
- **OneDrive support** — Sync status detection
- **ISO mounting** — Mount ISO files as virtual drives

### Performance & Cache
- **Multi-level cache** — Memory, disk (SQLite), and GPU textures
- **Async workers** — Background processing keeps UI responsive
- **Smart prefetch** — Predictive preloading of folders and files
- **UI virtualization** — Efficient rendering of large directories
- **Per-folder monitoring** — Default `notify` crate watcher with opt-in drive-wide `ReadDirectoryChangesW`

## Technologies

| Category | Technology | Version | Purpose |
|----------|-----------|---------|---------|
| **Language** | Rust | Edition 2021 | Performance and safety |
| **GUI** | eframe/egui | 0.31 | Modern immediate-mode GUI (features: `persistence`, `wgpu`) |
| **GPU Backend** | wgpu (via eframe) | 24.0.5 | Prefers native primary backends on Windows, with GL/ANGLE compatibility fallback and HighPerformance adapter preference |
| **Windows API** | windows-rs | 0.61.0 | Native Windows integration |
| **Video** | libmpv2 | 5.0.3 | High-performance video playback |
| **PDF** | pdfium (pdfium-render) | 0.8.37 | Native PDF rendering (requires pdfium.dll) |
| **Database** | SQLite (rusqlite) | 0.32 | Reliable persistence |
| **Images** | image crate | 0.25 | Image processing |
| **Parallelism** | rayon | 1.10 | Parallel processing |
| **IPC** | Named Pipes + bincode | 1.3 | App ↔ search service communication |
| **Service** | windows-service | 0.7 | Background indexing service |
| **i18n** | rust-i18n | 3 | Multi-language support (en, pt-BR) |

### Runtime Dependencies
- **libmpv-2.dll** — Required for video playback
- **pdfium.dll** — Required for PDF viewer
- **Video codecs** — Required for video thumbnail extraction (see [Video Thumbnail Codecs](#video-thumbnail-codecs) below)

## Installation

### Option 1: Build from Source
```bash
# Clone the repository
git clone <repository-url>
cd MTT-File-Manager-RUST

# Release build of the full workspace (main app + search service)
cargo build --release --workspace

# Run (release build opens without a console window on Windows)
.\target\release\mtt-file-manager.exe
```

### libmpv Setup
```powershell
# Download from: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Place libmpv-2.dll in the same directory as the executable
```

### pdfium Setup
```powershell
# build.rs tries to stage pdfium.dll automatically for local builds.
# Supported lookup locations:
#   .\vendor\pdfium.dll
#   .\vendor\pdfium\pdfium.dll
#   $env:PDFIUM_DYNAMIC_LIB_PATH\pdfium.dll

# If automatic staging does not happen, place pdfium.dll next to the executable
# before running the app or building the installer.
```

## Usage

### Keyboard Shortcuts
| Shortcut | Action |
|----------|--------|
| Ctrl+T | New tab |
| Ctrl+W | Close tab |
| Ctrl+C / Ctrl+V | Copy / Paste |
| Ctrl+X | Cut |
| Delete | Move to Recycle Bin |
| Shift+Delete | Permanent delete |
| F2 | Rename |
| F5 | Reload folder |
| Ctrl+Shift+F | Global search |
| Ctrl+L | Focus address bar |
| Ctrl+Mouse Wheel | Adjust thumbnail size |
| Alt+Enter | Properties |

### Supported Formats
- **Images**: JPG, PNG, GIF, WebP, BMP, TIFF, SVG — double-click opens the dedicated viewer
- **Videos**: MP4, MKV, AVI, MOV, WebM (requires libmpv)
- **Audio file detection / routing**: MP3, WAV, OGG, WMA, AAC, M4A, APE, MID, FLAC, ALAC, Opus, AIFF, WEBA
- **Audio playback**: Audio-only files open in the mpv-based player with a real-time waveform visualization
- **Audio metadata pipeline**: duration, codec, bitrate, channels, sample rate, artist, album, track title, genre, and year
- **Audio codec fallback detection**: AAC, MP3, FLAC, Opus, Vorbis, AC-3, E-AC-3, ALAC, PCM, WMA, DTS
- **PDFs**: Native viewer via pdfium (requires pdfium.dll)
- **GIFs**: Animated playback with play/pause controls

Additional formats may also work when Windows or installed codec handlers classify them as audio/video via `AssocGetPerceivedType`, but the list above reflects the explicit formats handled by the app's fast-path media routing and metadata code.

## Development

### Environment Setup
```bash
# Install Rust
rustup toolchain install stable
rustup default stable-msvc

# Verify
rustc --version
cargo --version
```

### Build & Run
```bash
# Development (entire workspace)
cargo build --workspace
cargo run

# Release build
cargo build --release --workspace

# Release build - search service only
cargo build --release -p mtt-search-service

# Run with logs
cargo run 2>&1 | Tee-Object "debug.log"

# Benchmarks
cargo bench
```

### Global Search Service
The search service is a separate workspace binary at `crates/mtt-search-service` and is also included automatically when you build the full workspace.

```powershell
# Build only the search service binary
cargo build --release -p mtt-search-service

# Binary output
.\target\release\mtt-search-service.exe
```

```powershell
# Install as service (requires Administrator)
.\target\release\mtt-search-service.exe install

# Start
sc.exe start MTTFileManagerSearch

# Check status
sc.exe query MTTFileManagerSearch

# Console mode (debug, no install needed)
.\target\release\mtt-search-service.exe run-console

# Uninstall
.\target\release\mtt-search-service.exe uninstall
```

Notes:
- The service installs as `LocalSystem` and exposes IPC over `\\.\pipe\MTTFileManagerSearch`.
- `cargo build --release --workspace` is the simplest way to produce both `mtt-file-manager.exe` and `mtt-search-service.exe` for local testing or packaging.

### Installer Build
The installer is generated with Inno Setup 6 and bundles the main app, search service, `libmpv-2.dll`, `pdfium.dll`, and the portable mpv configuration.

```powershell
# Install Inno Setup 6
winget install JRSoftware.InnoSetup

# From the repository root: build release artifacts + installer
.\installer\build_installer.ps1

# Reuse an existing release build
.\installer\build_installer.ps1 -SkipBuild

# Manual compilation (equivalent)
ISCC.exe .\installer\setup.iss
```

Artifacts explicitly prevalidated by `installer\build_installer.ps1`:
- `target\release\mtt-file-manager.exe`
- `target\release\mtt-search-service.exe`
- `target\release\libmpv-2.dll`
- `target\release\pdfium.dll`
- `appicon.ico`
- `mpv_ui\portable_config\mpv.conf`
- `mpv_ui\portable_config\scripts\`
- `mpv_ui\portable_config\scripts\autoload.lua`
- `mpv_ui\portable_config\scripts\modernH.lua`
- `mpv_ui\portable_config\scripts\vsr.lua`
- `mpv_ui\portable_config\script-opts\`
- `mpv_ui\portable_config\script-opts\osc.conf`

Installer behavior:
- Output is written to `installer\output\MTT-File-Manager-Setup-<version>.exe`
- The installer automatically installs and starts the `MTTFileManagerSearch` Windows service
- The installer warns if Microsoft Visual C++ Redistributable 2015-2022 (x64) is not detected

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
