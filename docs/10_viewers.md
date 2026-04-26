# Viewers and Media Player — MTT File Manager

## Overview

The `mtt-file-manager.exe` binary hosts four standalone viewer modes in addition to the main file manager. Each mode runs as a **separate process**, isolating memory, GPU textures, and native dependencies. The image, PDF, and text viewers share the lightweight `src/viewer_runtime.rs` setup. The video player uses its own native mpv window and does not use `viewer_runtime.rs`.

| Mode | CLI Flag | Code Location | Renderer | Process |
|------|----------|---------------|----------|---------|
| Image Viewer | `--image-viewer <path>` | `src/image_viewer/` | Glow (eframe) | Separate |
| PDF Viewer | `--pdf-viewer <path>` | `src/pdf_viewer/` | Glow (eframe) | Separate |
| Text Viewer | `--text-viewer <path>` | `src/text_viewer/` | Glow (eframe) | Separate |
| Video Player | `--video-player <path> [--position <s>] [--volume <v>]` | `src/video_player/` | mpv native (D3D11) | Separate |

## Shared Viewer Runtime

**Location**: `src/viewer_runtime.rs`

Provides a lightweight startup path for the image, PDF, and text viewers:

- **Read-only preferences**: Queries `app_state.db` via `Connection::open_with_flags(SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX)` to read `theme_mode` and `language` only
- **Glow renderer**: Configures `eframe::Renderer::Glow` instead of `Wgpu`
- **No window persistence**: `persist_window = false`, `multisampling = 0`, `depth_buffer = 0`, `stencil_buffer = 0`
- **Theme**: `is_saved_theme_dark()` reads the `"theme_mode"` preference key and returns `bool`
- **Locale**: `apply_saved_locale()` reads the `"language"` preference key and calls `rust_i18n::set_locale()`

The video player **does not** use `viewer_runtime.rs`; it initializes mpv directly with its own native window.

---

## 1. Image Viewer

**Location**: `src/image_viewer/`
**Entry point**: `image_viewer::run_standalone(PathBuf)`

### Architecture

The image viewer manages a sequence of images from the parent directory, with arrow-key navigation, a bottom filmstrip, zoom, and a GPU texture cache.

### Modules

| File | Responsibility |
|------|---------------|
| `mod.rs` | Process spawn, path validation, single-instance mutex, IPC forward to existing instance, window setup |
| `app/mod.rs` | `DedicatedImageViewerApp` — main state, navigation, keyboard handling, `eframe::App` implementation |
| `app/filmstrip.rs` | Bottom thumbnail strip rendering |
| `app/rendering.rs` | Top bar, bottom bar, and center viewport rendering |
| `app/gif_export.rs` | Animated GIF playback and image export/conversion |
| `cache.rs` | `WindowCache` (sliding-window `TextureHandle` cache) + `PrefetchEngine` (bounded-channel workers) |
| `indexer.rs` | `build_sequence()` — directory read, image extension filter, natural sort |
| `loader.rs` | Image decoding: memmap for files >1MB, EXIF orientation, WIC fallback |
| `metrics.rs` | Resource leak diagnostics/monitoring (handle, GDI, user, thread counters) |
| `ipc.rs` | Inter-process communication for forwarding images to an existing instance |

### Constants

```rust
const IMAGE_VIEWER_MUTEX_NAME: &str = "Global\\MTTFileManager_ImageViewer_SingleInstance\0";
const MAX_IMAGE_FILE_SIZE: u64 = 512 * 1024 * 1024; // 512 MB
const OPEN_REQUEST_DEBOUNCE: Duration = Duration::from_millis(700);
const DEFAULT_CACHE_RADIUS: usize = 3;
const MIN_ZOOM_FACTOR: f32 = 0.10;
const MAX_ZOOM_FACTOR: f32 = 8.0;
const MIN_NAVIGATE_INTERVAL: Duration = Duration::from_millis(20);
```

### Path Validation

`validate_image_path()` in `mod.rs` checks:
1. No null bytes
2. No path traversal components (`..`, `.`)
3. No UNC/network paths (`\\`, `//`, `\\?\UNC\`)
4. Extension must be a supported image type (via `crate::infrastructure::windows::is_image_extension`)
5. File must exist
6. File size must not exceed 512 MB

### Single Instance + IPC

- Uses a Windows named mutex (`Global\MTTFileManager_ImageViewer_SingleInstance`) to enforce a single instance
- If an instance already exists, the new open request is forwarded via IPC (`ipc::send_open_request`)
- Duplicate open requests for the same path within 700ms are suppressed

### Sliding-Window Cache

**Location**: `src/image_viewer/cache.rs`

- **Radius**: 3 (current image + 3 on each side)
- **Storage**: GPU `TextureHandle`s, not CPU RGBA buffers
- **Bounded channels**: `crossbeam::bounded(2*radius+1)` to prevent infinite job accumulation
- **Obsolete job cancellation**: Workers check an `AtomicUsize` center before decoding; jobs outside the current window are skipped
- **Tail-only fetch**: Navigation requests only the new edge image, not the full window
- **No blank flash**: The previous image remains visible until the new one is decoded and uploaded
- **Does not reuse file-manager thumbnails**: The viewer decodes its own full-resolution frames

### Startup Sequence

```
User double-clicks image
    ↓
main.rs detects --image-viewer flag
    ↓
run_standalone() validates path
    ↓
Tries IPC forward to existing instance; if unavailable, acquires single-instance mutex
    ↓
Starts IPC open-request server
    ↓
build_sequence() runs on background thread to enumerate and sort images
    ↓
Viewport starts hidden (with_visible(false)), size 1200×850
    ↓
PrefetchEngine initialized with worker_count = available_parallelism().clamp(1, 4)
    ↓
First frame ready → apply theme + reveal window
    ↓
On exit: cancel_pending_io_on_current_process_threads() + terminate_current_process(0)
```

### Keyboard Shortcuts

**Verified in `handle_shortcuts()` in `app/mod.rs`:**

| Key | Action |
|-----|--------|
| `Escape` | Close window |
| `←` / `A` / `Backspace` | Previous image |
| `→` / `D` / `Space` | Next image |

Note: The code does not define shortcuts for full-screen (F11), delete, copy, save-as, zoom keys, or rotation in `handle_shortcuts()`. Zoom controls (`MIN_ZOOM_FACTOR`, `MAX_ZOOM_FACTOR`) exist in state but the keyboard binding for zoom is not present in the read code.

### Window Properties

- Title: `imageviewer.title_with_file` (localized with filename)
- Size: 1200×850
- Decorations: `true` (native title bar)
- App ID: `mtt-file-manager-image-viewer`
- Icon: 256×256 PNG from embedded assets

---

## 2. PDF Viewer

**Location**: `src/pdf_viewer/`
**Entry point**: `pdf_viewer::run_standalone(PathBuf)`
**Runtime dependency**: `pdfium.dll`

### Architecture

Uses Google's PDFium via the `pdfium-render` crate (feature `thread_safe`). The render worker keeps a persistent Pdfium document handle and renders pages asynchronously.

### Modules

| File | Responsibility |
|------|---------------|
| `mod.rs` | Process spawn, path validation, standalone runner |
| `viewer_app.rs` | `PdfViewerApp` — state, theme, texture cache, zoom, scroll, `eframe::App` implementation |
| `renderer.rs` | Pdfium initialization (dynamic `pdfium.dll` loading), page rendering |
| `render_worker.rs` | Async worker with bounded channels keeping the Pdfium document open |
| `selection.rs` | Text selection support (highlight, copy) |
| `toolbar.rs` | Navigation toolbar (previous/next page, zoom, search) |

### Path Validation

`validate_pdf_path()` in `mod.rs` checks:
1. No null bytes
2. No path traversal components (`..`, `.`)
3. No UNC/network paths
4. Extension must be `.pdf` (case-insensitive)
5. File must exist
6. File size must not exceed 512 MB (`MAX_PDF_FILE_SIZE`)

On validation failure, `show_error_window()` displays the error in a minimal eframe window.

### Texture Cache

- **Memory budget**: `TEXTURE_MEMORY_BUDGET = 128 MB`
- **Eviction**: Furthest pages are evicted when the budget is exceeded
- **Async rendering**: The worker receives page requests via channel and returns GPU-ready `TextureHandle`s

### Startup

```
run_standalone()
    ↓
validate_pdf_path() (≤512MB, .pdf, no UNC/traversal)
    ↓
Viewport starts hidden, size 1024×768
    ↓
Load pdfium.dll dynamically (next to exe → system-wide)
    ↓
Create PdfViewerApp with render worker
    ↓
First frame applies theme + reveals window
```

### Window Properties

- Title: `pdfviewer.title_with_file` (localized with filename)
- Size: 1024×768
- Decorations: `true`
- App ID: `mtt-file-manager-pdf-viewer`
- Icon: 256×256 PNG from embedded assets

---

## 3. Text Viewer

**Location**: `src/text_viewer/`
**Entry point**: `text_viewer::run_standalone(PathBuf)`

### Architecture

Lightweight viewer for plain text, code, logs, and markup files. Optimized for low memory overhead on large files.

### Modules

| File | Responsibility |
|------|---------------|
| `mod.rs` | Process spawn, path validation, standalone runner |
| `viewer_app.rs` | `TextViewerApp` — state, monospace text rendering, vertical scroll, `eframe::App` implementation |

### Path Validation

`validate_text_path()` in `mod.rs` checks:
1. No null bytes
2. No path traversal components (`..`, `.`)
3. No UNC/network paths
4. Extension must be in the known text extension list (`TEXT_EXTENSIONS`)
5. File must exist
6. File size must not exceed 25 MB (`MAX_TEXT_FILE_SIZE`)

### Known Text Extensions

The `TEXT_EXTENSIONS` array in `mod.rs` contains:
- Plain text/logs: `txt`, `log`, `csv`, `tsv`, `nfo`, `diz`
- Config: `cfg`, `conf`, `ini`, `env`, `properties`, `toml`, `yaml`, `yml`, `editorconfig`, `gitignore`, `gitattributes`, `dockerignore`
- Data/markup: `json`, `xml`, `svg`, `html`, `htm`, `css`, `scss`, `sass`, `less`
- Code: `rs`, `py`, `js`, `ts`, `jsx`, `tsx`, `c`, `cpp`, `h`, `hpp`, `cs`, `java`, `go`, `rb`, `php`, `swift`, `kt`, `kts`, `scala`, `lua`, `r`, `m`, `mm`, `pl`, `pm`, `sql`
- Shell/scripting: `sh`, `bash`, `zsh`, `fish`, `bat`, `cmd`, `ps1`, `psm1`, `psd1`
- Documentation: `md`, `markdown`, `rst`, `tex`, `adoc`

### Memory Optimization

**Location**: `src/text_viewer/viewer_app.rs`

Instead of `Vec<String>` (one allocation per line), the viewer stores:

```rust
struct TextViewerApp {
    content: String,        // Full file contents
    line_offsets: Vec<u32>, // Byte offset of each line start
}
```

This avoids N String allocations and enables efficient line slicing via `&content[start..end]` without cloning.

### Rendering

- Monospace font layout
- Vertical scroll
- Line slicing from offsets (no string cloning)
- Dark/light theme applied on first frame

### Window Properties

- Title: `textviewer.title_with_file` (localized with filename)
- Size: 1024×768
- Decorations: `true`
- App ID: `mtt-file-manager-text-viewer`
- Icon: 256×256 PNG from embedded assets

---

## 4. Video Player (Media Player)

**Location**: `src/video_player/`
**Entry point**: `video_player::run_standalone(PathBuf, position: f64, volume: f32)`
**Runtime dependency**: `libmpv-2.dll`

### Architecture

The video player does not use egui/eframe for rendering. It creates a native mpv window (borderless) and renders via the D3D11 GPU pipeline.

### Modules

| File | Responsibility |
|------|---------------|
| `mod.rs` | `run_standalone()` — mpv initialization, event loop, shutdown, subtitle picker, window icon |

### Path Validation

`validate_video_path()` in `mod.rs` checks:
1. No null bytes
2. No path traversal components (`..`, `.`)
3. No UNC/network paths
4. Extension must be a supported video or audio type (via `is_video_extension` / `is_audio_extension`)
5. File must exist
6. File size must not exceed 50 GB (`MAX_VIDEO_FILE_SIZE`)

### mpv GPU Pipeline

After `mpv_initialize()`, the following properties are set programmatically:

```
vo = "gpu-next"
gpu-api = "d3d11"
gpu-context = "d3d11"
hwdec = "d3d11va"
```

This enables:
- **D3D11 GPU rendering** for video output
- **Hardware decoding** via D3D11 Video Acceleration
- **Borderless window** with OSC providing window controls

### Other mpv Runtime Properties

Set after initialization in `run_standalone()`:

| Property | Value | Purpose |
|----------|-------|---------|
| `force-window` | `true` | Show window even before video loads |
| `video-sync` | `"audio"` | Sync to audio clock |
| `interpolation` | `false` | Disable motion interpolation |
| `tscale` | `"linear"` | Temporal scaler |
| `framedrop` | `"vo"` | Drop frames at VO level |
| `keep-open` | `"always"` | Keep window open at EOF |
| `cache` | `"yes"` | Enable cache |
| `cache-secs` | `12.0` | Cache 12 seconds |
| `demuxer-readahead-secs` | `6.0` | Demuxer read-ahead |
| `demuxer-max-bytes` | `48 MB` | Max forward cache |
| `demuxer-max-back-bytes` | `12 MB` | Max backward cache |
| `volume` | `0–100` | Converted from CLI `volume` arg (×100, clamped) |
| `autofit` | `"55%x55%"` | Initial window size |
| `autofit-larger` | `"90%x90%"` | Max window size |
| `hidpi-window-scale` | `true` | Respect display scaling |
| `auto-window-resize` | `false` | Prevent VSR resize side effects |

### Audio Visualization

For audio-only files, mpv is configured with a `lavfi-complex` filter graph that renders a real-time white waveform on a black background:

```
[aid1]asplit[ao][a1];[a1]showwaves=s=1920x1080:mode=cline:rate=30:colors=white,format=pix_fmts=rgb24[vo]
```

### Event Loop

The main loop in `run_standalone()` blocks on `mpv.wait_event(1.0)` and handles:

| Event | Action |
|-------|--------|
| `Shutdown` | Break loop and exit |
| `FileLoaded` | Apply app icon to mpv window, log pipeline config, apply initial seek if `position > 0.5` |
| `ClientMessage` with `"open-subtitle-picker"` | Open native file dialog (`rfd::FileDialog`) for subtitle selection |
| `EndFile(reason)` | `Eof` → mark EOF flag; `Stop` → reset flags; `Quit` → exit; other after EOF → exit |
| `Err(e)` | Log warning |

### Subtitle Loading

`load_external_subtitle_for_standalone()` opens an `rfd::FileDialog` filtered to subtitle extensions (`srt`, `ass`, `ssa`, `vtt`, `sub`, `sup`, `idx`, `mks`) starting in the video's parent directory. On success, calls `mpv.command("sub-add", ...)` and displays a confirmation via `mpv.command("show-text", ...)`.

### OSC Configuration

The standalone player uses mpv's portable config from `mpv_ui/portable_config/`:
- `scripts/modernH.lua` — Custom OSC
- `scripts/vsr.lua` — VSR integration
- `scripts/autoload.lua` — Playlist autoload

The `osc-script-opts` are built with `build_mpv_osc_script_opts()`, which adds `osc-language` based on the saved locale (`pt-BR` or `en`).

### Window Icon

`set_mpv_window_icon()` loads the app icon from the executable's resources (via `LoadImageW` or `ExtractIconExW`) and applies it to the mpv window HWND via `SendMessageW(WM_SETICON)` and `SetClassLongPtrW`. If the HWND is not immediately available, it falls back to enumerating visible windows matching the current PID.

### Integration with File Manager

When a video is selected in the file manager:
1. The embedded preview panel uses `libmpv2` via `ui/components/mpv_preview/` (child HWND)
2. On double-click, `open_video_player()` spawns the standalone process with `--video-player <path> --position <s> --volume <v>`
3. The file manager tracks the child process (`video_player_process: Option<Child>`) and detects closure via `reap_video_player_process()`

---

## Viewer Comparison

| Aspect | Image Viewer | PDF Viewer | Text Viewer | Video Player |
|--------|-------------|------------|-------------|--------------|
| **Renderer** | Glow (eframe) | Glow (eframe) | Glow (eframe) | mpv native (D3D11) |
| **Uses viewer_runtime.rs** | Yes | Yes | Yes | No |
| **File size limit** | 512 MB | 512 MB | 25 MB | 50 GB |
| **GPU texture cache** | Sliding-window (radius 3) | 128 MB budget | None | N/A |
| **Workers** | PrefetchEngine (crossbeam) | Async render worker | None | N/A |
| **IPC instance reuse** | Yes (single instance + IPC) | No | No | No |
| **Text selection** | No | Yes | No | No |
| **Zoom** | Yes (0.10×–8.0×) | Yes | No | No |
| **Dark/light theme** | Yes | Yes | Yes | N/A |
| **Native title bar** | Yes (decorations=true) | Yes | Yes | Borderless (mpv OSC) |
| **Window size** | 1200×850 | 1024×768 | 1024×768 | 55%×55% (autofit) |

---

## Key Files by Viewer

### Image Viewer
- `src/image_viewer/mod.rs` — Entry point, validation, single instance, IPC
- `src/image_viewer/app/mod.rs` — App state, navigation, keyboard shortcuts, theme
- `src/image_viewer/cache.rs` — Sliding-window cache and PrefetchEngine
- `src/image_viewer/loader.rs` — Decoding (memmap, EXIF, WIC)
- `src/image_viewer/indexer.rs` — Directory enumeration and sorting

### PDF Viewer
- `src/pdf_viewer/mod.rs` — Entry point, validation
- `src/pdf_viewer/viewer_app.rs` — App state, texture cache, zoom, scroll
- `src/pdf_viewer/renderer.rs` — Pdfium initialization and page rendering
- `src/pdf_viewer/render_worker.rs` — Async render worker

### Text Viewer
- `src/text_viewer/mod.rs` — Entry point, validation, known extension list
- `src/text_viewer/viewer_app.rs` — App state, monospace rendering, line offsets

### Video Player
- `src/video_player/mod.rs` — Entry point, mpv initialization, event loop, subtitle picker, window icon

### Shared Runtime
- `src/viewer_runtime.rs` — Lightweight Glow setup, theme/locale loading for image/pdf/text viewers

## Runtime Dependencies

| Viewer | DLL / Dependency | Source |
|--------|-----------------|--------|
| PDF Viewer | `pdfium.dll` | Staged by `build.rs` (SHA-256 verified in release builds) |
| Video Player | `libmpv-2.dll` | Manual placement or bundled with installer |
| Image / Text | None external | Pure Rust dependencies only |
