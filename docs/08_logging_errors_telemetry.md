# Logging, Errors & Telemetry — MTT File Manager

## Logging

### Log Categories

The application uses `eprintln!` with category prefixes for structured logging:

| Prefix | Category | Description |
|--------|----------|-------------|
| `[INIT]` | Initialization | App startup events |
| `[CACHE]` | Cache | Cache hits, misses, evictions |
| `[THUMB]` | Thumbnails | Thumbnail generation events |
| `[THUMB_STAGE1]`–`[THUMB_STAGE5]` | Thumbnail stages | Per-stage extraction results |
| `[FILE_OP]` | File operations | Copy, move, delete operations |
| `[NAV]` | Navigation | Path navigation events |
| `[WORKER]` | Workers | Worker thread lifecycle |
| `[ERROR]` | Errors | Error conditions |
| `[WARN]` | Warnings | Warning conditions |
| `[PERF]` | Performance | Timing and performance data |
| `[WATCHER]` | File watcher | Filesystem change events |
| `[PDF]` | PDF viewer | PDF viewer events |
| `[MPV]` | Video player | mpv player events |
| `[IMAGE-VIEWER]` | Image viewer | Dedicated viewer events |
| `[FLOW]` | Flow control | Application flow tracking |
| `[STATE]` | State | State transition events |

### Capturing Logs

**Method 1: PowerShell script** (recommended)
```powershell
.\run_with_logs.ps1
# Output saved to: debug_metadata.log
```

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
pub enum AppError {
    Security(SecurityError),
    WindowsApi(String),
    Io(std::io::Error),
    ThumbnailExtraction { path: PathBuf, source: String },
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
| `safe_unwrap!(expr, fallback)` | Unwrap with fallback value instead of panic |
| `safe_expect!(expr, msg, fallback)` | Expect with fallback value instead of panic |

### Extension Traits

| Trait | Method | Purpose |
|-------|--------|---------|
| `OptionExt` | `ok_or_app_error(kind, msg)` | Convert `Option` to `AppResult` |
| `ResultExt` | `map_to_app_error(kind, msg)` | Convert `Result<T, E>` to `AppResult<T>` |

## Stack Traces

Enable stack traces on panics via environment variable:

```powershell
$env:RUST_BACKTRACE = 1       # Standard backtrace
$env:RUST_BACKTRACE = "full"  # Full backtrace with all frames
```

## Performance Metrics

The image viewer tracks performance metrics in `src/image_viewer/metrics.rs` for:
- Image decode time
- Cache hit/miss rates
- Prefetch worker utilization

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

