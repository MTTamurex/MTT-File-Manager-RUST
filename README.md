# MTT File Manager

**Native Windows file manager** built in Rust with a modern UI, advanced media preview, and deep Windows integration.

## Table of Contents

- [About](#about)
- [Key Features](#key-features)
- [Technologies](#technologies)
- [Requirements](#requirements)
- [Installation](#installation)
- [Usage](#usage)
- [Documentation](#documentation)
- [Development](#development)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)

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
| **GPU Backend** | wgpu (via eframe) | 24.0.5 | D3D12/Vulkan rendering with HighPerformance GPU preference |
| **Windows API** | windows-rs | 0.61.0 | Native Windows integration |
| **Video** | libmpv2 | 5.0.3 | High-performance video playback |
| **PDF** | pdfium (pdfium-render) | 0.8.37 | Native PDF rendering (requires pdfium.dll) |
| **Database** | SQLite (rusqlite) | 0.32 | Reliable persistence |
| **Images** | image crate | 0.25 | Image processing |
| **Parallelism** | rayon | 1.10 | Parallel processing |
| **IPC** | Named Pipes + bincode | 1.3 | App ↔ search service communication |
| **Service** | windows-service | 0.7 | Background indexing service |
| **i18n** | rust-i18n | 3 | Multi-language support (en, pt-BR) |

## Requirements

### Minimum
- **OS**: Windows 10 (Build 1903+) or Windows 11
- **CPU**: x64, 2+ cores
- **RAM**: 4 GB
- **Disk**: 100 MB + cache storage
- **GPU**: DirectX 12 or Vulkan capable (via wgpu)

### Recommended
- **OS**: Windows 11 (latest update)
- **CPU**: x64, 4+ cores
- **RAM**: 8 GB or more
- **Storage**: SSD for cache performance
- **GPU**: Dedicated GPU for video preview

### Runtime Dependencies
- **libmpv-2.dll** — Required for video playback
- **pdfium.dll** — Required for PDF viewer

## Installation

### Option 1: Build from Source
```bash
# Clone the repository
git clone <repository-url>
cd MTT-File-Manager-RUST

# Release build
cargo build --release --workspace

# Run (release build opens without a console window on Windows)
.\target\release\mtt-file-manager.exe
```

### libmpv Setup
```powershell
# Download from: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Place libmpv-2.dll in the same directory as the executable
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
- **PDFs**: Native viewer via pdfium (requires pdfium.dll)
- **GIFs**: Animated playback with play/pause controls

## Documentation

Access the [`docs/`](docs/) folder for complete technical documentation:

- **[Overview](docs/01_overview.md)** — Introduction and high-level architecture
- **[Build & Debug](docs/02_build_run_debug.md)** — Build, run, and debug instructions
- **[Architecture](docs/03_architecture.md)** — Detailed architecture and layers
- **[Module Map](docs/04_module_map.md)** — File structure and module responsibilities
- **[Dependencies](docs/05_dependencies_stack.md)** — Full technology stack
- **[Key Flows](docs/06_key_flows.md)** — How major features work
- **[Storage & Config](docs/07_storage_config.md)** — Data storage and configuration
- **[Logging & Errors](docs/08_logging_errors_telemetry.md)** — Logging and debugging
- **[Performance](docs/09_performance_optimizations.md)** — Performance optimizations

**Documentation index**: [docs/INDEX.md](docs/INDEX.md)

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

# Run with logs
cargo run 2>&1 | Tee-Object "debug.log"

# Benchmarks
cargo bench
```

### Global Search Service
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

### Project Structure
```
MTT-File-Manager-RUST/
├── Cargo.toml                        # Workspace root
├── src/                              # Main application
│   ├── app/                          # State and core logic
│   ├── application/                  # Business services
│   ├── domain/                       # Data models
│   ├── infrastructure/               # System integration
│   ├── ui/                           # User interface
│   ├── workers/                      # Background processing
│   ├── image_viewer/                 # Dedicated image viewer
│   ├── video_player/                 # Standalone video player
│   ├── pdf_viewer/                   # Native PDF viewer
│   └── tabs/                         # Tab management
├── crates/
│   ├── mtt-search-protocol/          # Shared IPC types (bincode)
│   └── mtt-search-service/           # Windows Service for file indexing
├── locales/                          # i18n (en.yml, pt-BR.yml)
├── docs/                             # Technical documentation
└── benches/                          # Benchmarks
```
