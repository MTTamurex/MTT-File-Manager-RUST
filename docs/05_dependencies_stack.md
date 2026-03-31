# Dependency Stack — MTT File Manager

## Main Crate: `mtt-file-manager`

### GUI Framework

| Crate | Version | Purpose |
|-------|---------|---------|
| eframe | 0.31 | egui framework with windowing (features: `persistence`, `wgpu`) |
| wgpu | 24.0.5 (via eframe) | GPU rendering backend (D3D12/Vulkan) with HighPerformance preference |

### Concurrency & Channels

| Crate | Version | Purpose |
|-------|---------|---------|
| rayon | 1.10 | Data-parallel processing |
| crossbeam-channel | 0.5.15 | High-performance MPSC channels |
| once_cell | 1.19 | Lazy static initialization |

### File I/O

| Crate | Version | Purpose |
|-------|---------|---------|
| walkdir | 2.5 | Recursive directory traversal |
| notify | 6.1.1 | Filesystem event watcher (optional, `notify-watcher` feature) |
| memmap2 | 0.9 | Memory-mapped file I/O for large images |
| tempfile | 3.10 | Temporary file creation |

### Caching & Data Structures

| Crate | Version | Purpose |
|-------|---------|---------|
| lru | 0.12 | LRU cache |
| dashmap | 5.5 | Concurrent hash map |
| rustc-hash | 2.0 | Fast FxHashSet/FxHashMap |
| fxhash | 0.2.1 | Fast hashing for PathBuf keys |
| blake3 | 1.5 | 128-bit hash for cache keys |

### Image Processing

| Crate | Version | Purpose |
|-------|---------|---------|
| image | 0.25 | Image decoding/encoding (features: `webp`, `gif`) |
| webp | 0.3 | Lossy WebP compression for thumbnails |
| resvg | 0.44 | SVG rendering |
| usvg | 0.44 | SVG parsing |
| tiny-skia | 0.11 | Rasterization backend for SVG |
| kamadak-exif | 0.5 | EXIF metadata extraction from JPEG |

### Media

| Crate | Version | Purpose |
|-------|---------|---------|
| libmpv2 | 5.0.3 | mpv video player bindings |
| pdfium-render | 0.8.37 | PDF rendering (via pdfium.dll, feature: `thread_safe`) |

### Serialization

| Crate | Version | Purpose |
|-------|---------|---------|
| serde | 1.0 | Serialization framework (workspace, feature: `derive`) |
| serde_json | 1.0 | JSON serialization |
| bincode | 1.3 | Binary serialization for IPC (workspace) |

### Database

| Crate | Version | Purpose |
|-------|---------|---------|
| rusqlite | 0.32 | SQLite bindings (workspace, feature: `bundled`) |

### System & Platform

| Crate | Version | Purpose |
|-------|---------|---------|
| dirs | 5.0 | Platform-specific directory paths |
| clipboard-win | 5.4 | Windows clipboard (CF_HDROP format) |
| rfd | 0.15 | Native file dialogs |
| raw-window-handle | 0.6 | Raw window handle abstraction |

### Logging

| Crate | Version | Purpose |
|-------|---------|---------|
| log | 0.4 | Logging facade |
| env_logger | 0.11 | Level-filtered logging output |

### Internationalization

| Crate | Version | Purpose |
|-------|---------|---------|
| rust-i18n | 3 | Compile-time i18n with YAML locale files |

### Sorting

| Crate | Version | Purpose |
|-------|---------|---------|
| natord | 1.0 | Natural (human-friendly) sort ordering |

### Error Handling

| Crate | Version | Purpose |
|-------|---------|---------|
| thiserror | 2.0 | Derive macro for error types |

### Windows API

| Crate | Version | Purpose |
|-------|---------|---------|
| windows | 0.61.0 | Windows API bindings (35 feature flags) |

**Windows feature flags used:**
- **Shell**: `Win32_UI_Shell`, `Win32_UI_Shell_Common`, `Win32_UI_Shell_PropertiesSystem`
- **COM**: `Win32_System_Com`, `Win32_System_Com_StructuredStorage`
- **Clipboard**: `Win32_System_DataExchange`
- **Memory**: `Win32_System_Memory`
- **Registry**: `Win32_System_Registry`
- **Graphics**: `Win32_Graphics_Gdi`, `Win32_Graphics_Imaging`, `Win32_Graphics_Dwm`
- **File System**: `Win32_Storage_FileSystem`, `Win32_Storage_Vhd`
- **Process/Threading**: `Win32_System_ProcessStatus`, `Win32_System_Threading`, `Win32_System_LibraryLoader`
- **Input**: `Win32_UI_Input_KeyboardAndMouse`
- **Media**: `Win32_Media_MediaFoundation`
- **Devices**: `Win32_Devices_DeviceAndDriverInstallation`
- **I/O**: `Win32_System_Ioctl`, `Win32_System_IO`, `Win32_System_Pipes`
- **Windows**: `Win32_Foundation`, `Win32_UI_WindowsAndMessaging`, `Win32_System_WindowsProgramming`, `Win32_System_Time`
- **Network**: `Win32_NetworkManagement_WNet`
- **Globalization**: `Win32_Globalization`
- **Security**: `Win32_Security`
- **Search**: `Win32_System_Search_Common`
- **Variant**: `Win32_System_Variant`
- **UWP/WinRT**: `Foundation`, `Data_Pdf`, `Storage`, `Storage_Streams`

### Local Dependencies

| Crate | Source | Purpose |
|-------|--------|---------|
| mtt-search-protocol | path: `crates/mtt-search-protocol` | Shared IPC types |

## Crate: `mtt-search-protocol`

| Crate | Version | Purpose |
|-------|---------|---------|
| serde | workspace | Serialization |
| bincode | workspace | Binary IPC encoding |

## Crate: `mtt-search-service`

| Crate | Version | Purpose |
|-------|---------|---------|
| mtt-search-protocol | local path | Shared IPC types |
| windows-service | 0.7 | Windows Service Control Manager integration |
| rusqlite | workspace (bundled) | SQLite persistence for file index |
| serde | workspace | Serialization |
| bincode | workspace | Binary IPC encoding |
| windows | 0.61.0 | Windows API (8 features: Foundation, FileSystem, Console, Ioctl, IO, Pipes, Security, Threading) |

## Build Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| winresource | 0.1 | Embed Windows icon and metadata in executable |

## Dev Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| criterion | 0.5 | Benchmarking framework |

## Runtime Dependencies

| Dependency | Required | Purpose |
|-----------|----------|---------|
| libmpv-2.dll | For video playback | mpv shared library |
| pdfium.dll | For PDF viewer | Google's PDF rendering library |
| Windows 10+ | Always | Native Windows API integration |

## Feature Flags

```toml
[features]
default = ["notify-watcher"]
notify-watcher = ["notify"]
```

- **`notify-watcher`** (default) — Enables the `notify` crate as a fallback filesystem watcher for UNC/network paths. The primary monitoring uses the native Drive Watcher (`ReadDirectoryChangesW`).

## Build Profiles

```toml
[profile.release]
opt-level = 3       # Maximum optimization
lto = true          # Link-Time Optimization
codegen-units = 1   # Single codegen unit for best optimization
```

## Dependency Auditing

```bash
# View dependency tree
cargo tree

# Check for vulnerabilities
cargo install cargo-audit
cargo audit

# Check for outdated dependencies
cargo install cargo-outdated
cargo outdated
```

