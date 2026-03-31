# Build, Run & Debug — MTT File Manager

## Prerequisites

### Rust Toolchain
```powershell
# Install via rustup (Windows)
winget install Rustlang.Rustup
```

### MSVC Build Tools
- **Visual Studio Build Tools** or **Visual Studio Community**
- Required components:
  - MSVC v143 — VS 2022 C++ x64/x86 build tools
  - Windows 10/11 SDK

### System Dependencies
- **Windows 10** or **Windows 11**
- **libmpv-2.dll** — Required for video playback
- **pdfium.dll** — Required for PDF viewer

### Optional Dependencies
```powershell
# libmpv (for video playback)
# Download from: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Place libmpv-2.dll in the same directory as the executable or in PATH
```

## Building

### Development Build
```bash
# Clone the repository
git clone <repository-url>
cd MTT-File-Manager-RUST

# Build entire workspace (app + search service)
cargo build --workspace

# Build only the main app
cargo build -p mtt-file-manager

# Run in debug mode
cargo run
```

### Release Build
```bash
# Release build of entire workspace
cargo build --release --workspace

# Release build — app only
cargo build --release -p mtt-file-manager

# Release build — search service only
cargo build --release -p mtt-search-service

# Run the app (release build opens without a console window on Windows)
.\target\release\mtt-file-manager.exe

# Run the search service in console mode (debug)
.\target\release\mtt-search-service.exe run-console
```

### Feature Flags
```bash
# Default build (Drive Watcher + fallback notify-watcher)
cargo build

# Build without optional features
# (disables notify fallback for UNC/network paths; native Drive Watcher remains active)
cargo build --no-default-features
```

Available features:
- **`notify-watcher`** (default) — Enables `notify` crate as fallback watcher for UNC/network paths
- Primary filesystem monitoring uses the native Drive Watcher (`ReadDirectoryChangesW` on the drive root)

### Build Profiles

**Dev** (default):
```toml
[profile.dev]
opt-level = 0
debug = true
debug-assertions = true
overflow-checks = true
```

**Release** (configured in Cargo.toml):
```toml
[profile.release]
opt-level = 3       # Maximum optimization
lto = true          # Link-Time Optimization
codegen-units = 1   # Single codegen unit for best optimization
```

## Global Search Service

The search service (`mtt-search-service`) runs as a Windows Service with hybrid per-volume indexing:
- **NTFS/ReFS**: USN Journal (full MFT scan on first run + incremental loop)
- **Non-USN (exFAT/FAT32/FUSE/CryptoFS)**: Full-tree scan with SQLite cache + periodic re-scan

```powershell
# Install as service (requires Administrator PowerShell)
.\target\release\mtt-search-service.exe install

# Start the service
sc.exe start MTTFileManagerSearch

# Check status
sc.exe query MTTFileManagerSearch

# Stop the service
sc.exe stop MTTFileManagerSearch

# Uninstall the service
.\target\release\mtt-search-service.exe uninstall
```

**Non-USN update cadence**:
- 30s for virtual filesystems (`fuse`, `cryptofs`, `dokan`, `winfsp`)
- 120s for physical volumes without USN (e.g., exFAT/FAT32)

**Note**: Administrator privileges and `LocalSystem` runtime are required for USN access (`FSCTL_*`).

### IPC Hardening (Optional)

To reduce status metadata exposure in restricted environments:

```powershell
$env:MTT_SEARCH_REDACT_STATUS_METRICS = "1"
sc.exe stop MTTFileManagerSearch
sc.exe start MTTFileManagerSearch
```

With this flag, the service returns `redacted` volume states and zeroed counts in `GetStatus` responses while search/pagination remains functional.

## Running with Logs

Release builds of `mtt-file-manager.exe` use the Windows GUI subsystem, so launching the executable directly does not open an extra console window. For diagnostics, run it explicitly from PowerShell so stdout/stderr can be captured.

### Method 1: PowerShell Script (Recommended)
```powershell
.\run_with_logs.ps1
# Logs are saved to: debug_metadata.log
```

### Method 1B: Dedicated diagnostic console window
```cmd
.\open_diagnostic_console.cmd
```

This opens a separate PowerShell window only when you want to inspect logs.

### Method 2: Manual Redirection
```powershell
# Redirect stderr to file and console
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "app_debug.log"

# Display in console only
.\target\release\mtt-file-manager.exe 2>&1

# Errors only
.\target\release\mtt-file-manager.exe 2> "errors.log"
```

### Method 3: Filtered Output
```powershell
# Color-coded by category
.\target\release\mtt-file-manager.exe 2>&1 | ForEach-Object {
    if ($_ -match "ERROR") { Write-Host $_ -ForegroundColor Red }
    elseif ($_ -match "WARN") { Write-Host $_ -ForegroundColor Yellow }
    elseif ($_ -match "THUMB") { Write-Host $_ -ForegroundColor Cyan }
    else { Write-Host $_ -ForegroundColor Gray }
}

# Filter by category
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|PERF"
```

## Debug & Profiling

### VS Code Debugging
1. Install the `rust-analyzer` extension
2. Create `.vscode/launch.json`:
```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "lldb",
            "request": "launch",
            "name": "Debug MTT File Manager",
            "cargo": {
                "args": ["build", "--bin", "mtt-file-manager"],
                "filter": {
                    "name": "mtt-file-manager",
                    "kind": "bin"
                }
            },
            "args": [],
            "cwd": "${workspaceFolder}"
        }
    ]
}
```

### Flamegraph Profiling
```bash
cargo install flamegraph
cargo flamegraph --bin mtt-file-manager
# Output: flamegraph.svg
```

### Benchmarks
```bash
# Run all benchmarks
cargo bench

# Specific benchmark
cargo bench --bench shell_ops_blocking
cargo bench --bench image_viewer_decode
```

### Dependency Auditing
```bash
cargo tree                    # Dependency tree
cargo install cargo-audit
cargo audit                   # Vulnerability check
cargo install cargo-outdated
cargo outdated                # Available updates
```

## Troubleshooting

### "libmpv-2.dll not found"
```powershell
Copy-Item "path\to\libmpv-2.dll" -Destination ".\target\release\"
# Or add to PATH
$env:PATH += ";C:\Path\To\libmpv"
```

### Slow Build
```bash
cargo build --release -j 8    # Parallel compilation
```

### Windows API Compilation Error
Ensure Windows SDK is installed via Visual Studio Installer:
- MSVC v143
- Windows 10/11 SDK

## Useful Commands

```bash
# Development
cargo build --workspace       # Build all crates
cargo run                     # Run app (debug)
cargo check                   # Fast type-check without building
cargo check -p mtt-file-manager  # Check specific package

# Quality
cargo fmt                     # Format code
cargo clippy                  # Lint

# Production
cargo build --release --workspace
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "debug.log"
.\target\release\mtt-search-service.exe run-console

# Cleanup
cargo clean
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

## Environment Variables

```powershell
$env:RUST_BACKTRACE=1                    # Enable backtraces on panic
$env:RUST_BACKTRACE="full"               # Full backtraces
$env:RUST_LOG="debug"                    # Debug logging
$env:RUST_LOG="mtt_file_manager=debug"   # Module-specific logging
$env:CARGO_INCREMENTAL=1                 # Incremental compilation
```

## VS Code Settings

```json
{
    "rust-analyzer.cargo.features": ["notify-watcher"],
    "rust-analyzer.checkOnSave.command": "clippy",
    "rust-analyzer.cargo.buildScripts.enable": true
}
```

