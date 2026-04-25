# Logging, Errors & Telemetry — MTT File Manager

## Logging

### Log Categories

The main application uses the `log` crate (`log::info!`, `log::warn!`, `log::error!`, `log::debug!`) with `env_logger` as the backend. The search service uses `eprintln!` for output. Messages are tagged with bracket prefixes for filtering.

#### Main Application (`src/`) — via `log::` macros

| Prefix | Category | Description |
|--------|----------|-------------|
| `[INIT]` | Initialization | App startup events |
| `[LIFECYCLE]` | Lifecycle | Focus/minimize restore, idle detection, shutdown |
| `[EXIT]` | Shutdown | App exit and cleanup |
| `[NAV]` | Navigation | Path navigation events |
| `[PERF]` | Performance | Frame timing and performance data |
| `[PERF-MSG]` | Message processing | Per-frame message handler timing |
| `[PERF-ICON]` | Icon performance | Icon extraction timing |
| `[PERF-THUMB-UPLOAD]` | Thumbnail upload | GPU texture upload budgeting per frame |
| `[WATCHER]` | Watcher routing | Watcher strategy selection and lifecycle |
| `[FS-WATCH]` | Filesystem events | Watcher event processing and auto-reload |
| `[NOTIFY-WATCHER]` | Notify watcher | Per-folder `notify` crate watcher events |
| `[DRIVE-WATCHER]` | Drive watcher | Drive-wide `ReadDirectoryChangesW` events |
| `[DRIVE-WATCHER-MGR]` | Drive watcher manager | Multi-drive watcher management |
| `[MTIME-CHECK]` | Modification time | File modification time verification |
| `[MTIME-SCHED]` | Mtime scheduling | Scheduled modification time checks |
| `[GLOBAL-SEARCH]` | Global search | Search overlay and IPC events |
| `[SESSION-SEARCH]` | Session search | User session search index events |
| `[THUMB WORKER]` | Thumbnail worker | Thumbnail request processing |
| `[Thumbnail]` | Thumbnail extraction | Thumbnail extraction pipeline events |
| `[ThumbnailWorker]` | Thumbnail lifecycle | Thumbnail worker lifecycle |
| `[FILE-OP]` | File operations | File operation worker events |
| `[FileOps]` | File operations | Copy, move, delete, paste operations |
| `[SECURITY]` | Security | Security validation events |
| `[FOLDER-LOCK]` | Folder locks | Per-folder view preference events |
| `[PINNED]` | Pinned folders | Pinned folder persistence |
| `[PinnedFolders]` | Pinned folders | Pinned folder operations |
| `[Cache]` | Cache | Cache initialization and disk cache events |
| `[DISK-CACHE]` | Disk cache | SQLite disk cache operations |
| `[APP-STATE]` | App state | SQLite app-state operations and fallback handling |
| `[GC]` | Garbage collection | Cache garbage collection |
| `[IconDiskCache]` | Icon cache | Icon disk cache operations |
| `[Config]` | Configuration | Virtual drive and settings events |
| `[Migration]` | SQLite migration | Legacy table migration from `thumbnails.db` to `app_state.db` |
| `[DEBUG]` | Debug | Miscellaneous debug information |
| `[COVER]` | Folder covers | Folder cover composition events |
| `[DRIVE-REFRESH]` | Drive refresh | Drive list refresh events |
| `[GifPlayer]` | GIF playback | GIF animation events |
| `[IMAGE-VIEWER]` | Image viewer | Dedicated image viewer events |
| `[VIDEO-PLAYER]` | Video player | Standalone video player events |
| `[VIDEO]` | Video preview | Video preview lifecycle |
| `[PDF-VIEWER]` | PDF viewer | PDF viewer events |
| `[PDF-RENDER]` | PDF render | PDF page rendering events |
| `[MPV]` | mpv | mpv player events |
| `[MpvPreview]` | mpv preview | Embedded mpv preview events |
| `[ShellMenu]` | Shell menu | Native context menu events |
| `[ShellMenuWorker]` | Shell menu worker | Background shell menu extraction |
| `[SHELL-FOLDER]` | Shell folders | Shell special folder resolution |
| `[OneDrive]` | OneDrive | OneDrive path detection |
| `[ISO]` | ISO mount | ISO file mounting |
| `[METADATA]` | Metadata | File metadata extraction |
| `[Lixeira]` | Recycle Bin | Recycle Bin enumeration |
| `[RECYCLE]` | Recycle Bin | Recycle Bin operation events |
| `[RECYCLE BIN]` | Recycle Bin | Recycle Bin view setup |
| `[PROPERTIES]` | Properties | File properties dialog |
| `[VOLUME-RENAME]` | Volume rename | Drive volume label changes |
| `[HardwareDetection]` | Hardware | GPU hardware acceleration detection |
| `[device_change]` | Device change | Device plug/unplug monitoring |
| `[ArchiveExtract]` | Archive extraction | Native archive extraction fallback events |
| `[ArchiveExtract/ZIP]` | ZIP extraction | ZIP archive extraction details |
| `[ArchiveExtract/7z]` | 7z extraction | 7z archive extraction details |
| `[ArchiveExtract/RAR]` | RAR extraction | RAR archive extraction details |
| `[ArchiveExtract/TAR]` | TAR extraction | TAR archive extraction details |
| `[PASTE-DIAG]` | Paste diagnostics | Clipboard paste operation diagnostics |
| `[STARTUP]` | Startup diagnostics | Early startup events (GPU backend, storage cleanup) |

#### Search Service (`crates/mtt-search-service/`) — via `eprintln!`

| Prefix | Category | Description |
|--------|----------|-------------|
| `[SERVICE]` | Service | Service lifecycle and SCM events |
| `[IPC]` | IPC | Named Pipe server events |
| `[DB]` | Database | SQLite persistence events |
| `[SCAN]` | Scanning | Volume scanning events |
| `[USN]` | USN Journal | USN Journal indexing events |
| `[INDEX-DB]` | Index loading | Binary/SQLite index load events |

### Log Level Behavior

On Windows release builds launched without a console (e.g., from a desktop shortcut), the default log level is raised to `warn,mtt_file_manager=warn` to prevent background worker threads from generating heavy heap-allocator contention through failed stderr writes. When launched from a terminal (`cargo run`, PowerShell), the default is `warn,mtt_file_manager=info`.

### Capturing Logs

Release builds of `mtt-file-manager.exe` use the Windows GUI subsystem, so double-clicking the executable does not open a log console. For analysis, start the app from PowerShell or use the helper script below.

**Method 1: PowerShell script** (recommended)
```powershell
.\run_with_logs.ps1
# Output saved to: debug_metadata.log
```

**Method 1B: Dedicated diagnostic console window**
```cmd
.\open_diagnostic_console.cmd
```

Use this when you want the release app to stay silent by default but still have a one-click console for troubleshooting.

**Method 2: Manual redirection**
```powershell
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "app_debug.log"
```

**Method 3: Filtered output**
```powershell
# Filter by category
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|PERF"

# Color-coded output
.\target\release\mtt-file-manager.exe 2>&1 | ForEach-Object {
    if ($_ -match "ERROR") { Write-Host $_ -ForegroundColor Red }
    elseif ($_ -match "WARN") { Write-Host $_ -ForegroundColor Yellow }
    elseif ($_ -match "THUMB") { Write-Host $_ -ForegroundColor Cyan }
    else { Write-Host $_ -ForegroundColor Gray }
}
```

## Error Handling

### AppError Type

Defined in `src/domain/errors.rs`:

```rust
#[derive(Error, Debug)]
pub enum AppError {
    Security(#[from] crate::infrastructure::security::SecurityError),
    WindowsApi(String),
    Io(#[from] std::io::Error),
    ThumbnailExtraction { path: PathBuf, #[source] source: Box<dyn std::error::Error + Send + Sync> },
    FileOperation(String),
    InvalidState(String),
    Config(String),
    Worker(String),
    UiRendering(String),
}

pub type AppResult<T> = Result<T, AppError>;
```

### Helper Functions

| Function | Creates |
|----------|---------|
| `windows_error(msg)` | `AppError::WindowsApi` |
| `file_operation_error(msg)` | `AppError::FileOperation` |
| `invalid_state_error(msg)` | `AppError::InvalidState` |
| `config_error(msg)` | `AppError::Config` |
| `worker_error(msg)` | `AppError::Worker` |
| `ui_rendering_error(msg)` | `AppError::UiRendering` |

### Safety Macros

| Macro | Purpose |
|-------|---------|
| `safe_unwrap!(expr, context)` | Unwrap `Result` with error propagation (returns `Err`) |
| `safe_unwrap!(expr, context, default)` | Unwrap `Result` with fallback value on failure |
| `safe_expect!(expr, message)` | Unwrap `Option` or return `Err(AppError::InvalidState)` |

### Extension Traits

| Trait | Method | Purpose |
|-------|--------|---------|
| `OptionExt` | `ok_or_app_error(context)` | Convert `Option` to `AppResult` |
| `ResultExt` | `map_to_app_error(context)` | Convert `Result<T, E>` to `AppResult<T>` |

## Stack Traces

Enable stack traces on panics via environment variable:

```powershell
$env:RUST_BACKTRACE = 1       # Standard backtrace
$env:RUST_BACKTRACE = "full"  # Full backtrace with all frames
```

## Performance Metrics

The image viewer tracks performance metrics in `src/image_viewer/metrics.rs` for:
- Image decode time (count + average)
- GPU texture upload time (count + average)

## Debugging Specific Issues

### Thumbnail problems
```powershell
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|ERROR" | Tee-Object "thumb_debug.log"
```

### Watcher problems
```powershell
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "WATCHER|ERROR"
```

### Navigation problems
```powershell
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "NAV|FLOW|ERROR"
```

### Archive extraction problems
```powershell
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "ArchiveExtract|ERROR"
```

### Global search problems
```powershell
# Check service status
sc.exe query MTTFileManagerSearch

# Run service in console mode for logs
.\target\release\mtt-search-service.exe run-console
```

## Diagnostic Script

Collect system information for debugging:

```powershell
# System info
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber
Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores

# Check running processes
Get-Process mtt-file-manager -ErrorAction SilentlyContinue | Select-Object CPU, WorkingSet64

# Check cache directory
Get-ChildItem "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse | Measure-Object -Property Length -Sum

# Check search service
sc.exe query MTTFileManagerSearch
```
