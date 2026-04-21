# Getting Started

<cite>
**Referenced Files in This Document**
- [README.md](file://README.md)
- [docs/02_build_run_debug.md](file://docs/02_build_run_debug.md)
- [docs/05_dependencies_stack.md](file://docs/05_dependencies_stack.md)
- [Cargo.toml](file://Cargo.toml)
- [build.rs](file://build.rs)
- [src/main.rs](file://src/main.rs)
- [src/viewer_runtime.rs](file://src/viewer_runtime.rs)
- [crates/mtt-search-service/src/main.rs](file://crates/mtt-search-service/src/main.rs)
- [crates/mtt-search-service/Cargo.toml](file://crates/mtt-search-service/Cargo.toml)
- [installer/build_installer.ps1](file://installer/build_installer.ps1)
- [run_with_logs.ps1](file://run_with_logs.ps1)
- [open_diagnostic_console.cmd](file://open_diagnostic_console.cmd)
- [.cargo/config.toml](file://.cargo/config.toml)
- [app.manifest](file://app.manifest)
- [security_verify.ps1](file://security_verify.ps1)
- [virtual_drive_config.json](file://virtual_drive_config.json)
</cite>

## Update Summary
**Changes Made**
- Completely rewritten the Getting Started section to reflect the comprehensive new documentation in `docs/02_build_run_debug.md`
- Updated all installation and build instructions to match the new standardized format
- Enhanced troubleshooting section with detailed GPU backend diagnostics
- Added comprehensive runtime dependency management information
- Updated development environment setup with detailed verification steps
- Expanded logging and diagnostics section with practical scripts and environment variables

## Table of Contents
1. [Introduction](#introduction)
2. [Prerequisites](#prerequisites)
3. [Development Environment Setup](#development-environment-setup)
4. [Installation Options](#installation-options)
5. [Build Instructions](#build-instructions)
6. [Runtime Dependencies](#runtime-dependencies)
7. [Execution Modes](#execution-modes)
8. [Global Search Service](#global-search-service)
9. [Logging and Diagnostics](#logging-and-diagnostics)
10. [Troubleshooting Guide](#troubleshooting-guide)
11. [Performance Considerations](#performance-considerations)
12. [Security and Verification](#security-and-verification)
13. [Appendices](#appendices)

## Introduction
This guide provides comprehensive instructions for installing, building, and running MTT File Manager from source or with pre-built releases. The documentation covers:

- Complete development environment setup with Rust toolchain and Windows build tools
- Multiple installation approaches including source building and pre-built releases
- Detailed build instructions for different scenarios and configurations
- Runtime dependency management for libmpv-2.dll and pdfium.dll
- Various execution modes including main application, standalone viewers, and development logging
- Comprehensive troubleshooting for common setup issues and verification steps

## Prerequisites

### Rust Toolchain
Install Rust via rustup for Windows development:

```powershell
# Install via rustup (Windows)
winget install Rustlang.Rustup
```

### MSVC Build Tools
Required components for Windows development:
- **Visual Studio Build Tools** or **Visual Studio Community**
- **MSVC v143** — VS 2022 C++ x64/x86 build tools
- **Windows 10/11 SDK**

### System Dependencies
- **Windows 10** or **Windows 11** operating system
- **libmpv-2.dll** — Required for video playback functionality
- **pdfium.dll** — Required for PDF viewer capabilities

### GPU Backend Notes
The application uses different rendering backends depending on the component:

- **Main desktop window** uses `eframe` with the `wgpu` renderer
- **Current startup configuration** prefers native primary `wgpu` backends on Windows
- **HighPerformance adapter selection** for the main application
- **Dedicated GPU is not mandatory** — hybrid systems prefer discrete adapters when available
- **GL backend compatibility** — Windows OpenGL/ANGLE path when preferred native backend is unavailable
- **Standalone viewers** use separate `glow` renderer path via `src/viewer_runtime.rs`
- **Renderer initialization failure** — main file-manager window requires wgpu initialization

### Optional Dependencies
```powershell
# libmpv (for video playback)
# Download from: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Place libmpv-2.dll in the same directory as the executable or in PATH
```

**Section sources**
- [docs/02_build_run_debug.md:3-29](file://docs/02_build_run_debug.md#L3-L29)
- [docs/02_build_run_debug.md:30-36](file://docs/02_build_run_debug.md#L30-L36)
- [docs/02_build_run_debug.md:37-47](file://docs/02_build_run_debug.md#L37-L47)

## Development Environment Setup

### Installing Rust Toolchain
```bash
# Install Rust via rustup
rustup toolchain install stable
rustup default stable-msvc
```

### Verifying Installation
```bash
# Verify Rust installation
rustc --version
cargo --version
```

### Required Visual Studio Components
Ensure the following components are installed:
- MSVC v143 — VS 2022 C++ x64/x86 build tools
- Windows 10/11 SDK

**Section sources**
- [docs/02_build_run_debug.md:5-16](file://docs/02_build_run_debug.md#L5-L16)
- [docs/02_build_run_debug.md:365-369](file://docs/02_build_run_debug.md#L365-L369)

## Installation Options

### Pre-built Releases
The installer bundles all necessary components:
- Main application executable (`mtt-file-manager.exe`)
- Search service executable (`mtt-search-service.exe`)
- Runtime dependencies (`libmpv-2.dll`, `pdfium.dll`)
- Portable mpv configuration
- License and notice files
- Automatically installs and starts the Windows service

### Build from Source
Use the workspace build to produce both executables and optionally the installer:

```bash/bash
# Clone repository
git clone <repository-url>
cd MTT-File-Manager-RUST

# Build entire workspace (app + search service)
cargo build --workspace

# Build only the main app
cargo build -p mtt-file-manager

# Build only the search service
cargo build -p mtt-search-service
```

**Section sources**
- [README.md:73-104](file://README.md#L73-L104)
- [docs/02_build_run_debug.md:58-77](file://docs/02_build_run_debug.md#L58-L77)
- [installer/build_installer.ps1:48-81](file://installer/build_installer.ps1#L48-L81)

## Build Instructions

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

# Run standalone viewer modes from the same executable
cargo run -- --image-viewer "C:\path\to\image.jpg"
cargo run -- --pdf-viewer "C:\path\to\file.pdf"
cargo run -- --text-viewer "C:\path\to\file.txt"
cargo run -- --video-player "C:\path\to\file.mp4"
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

**Section sources**
- [docs/02_build_run_debug.md:58-131](file://docs/02_build_run_debug.md#L58-L131)
- [docs/02_build_run_debug.md:99-112](file://docs/02_build_run_debug.md#L99-L112)
- [docs/02_build_run_debug.md:113-131](file://docs/02_build_run_debug.md#L113-L131)
- [README.md:143-165](file://README.md#L143-L165)

## Runtime Dependencies

### libmpv-2.dll
Required for video playback functionality. Place it next to the executable or in PATH:
- **Location**: Same directory as the executable or in system PATH
- **Purpose**: High-performance video playback engine
- **Source**: Download from mpv-player-windows releases

### pdfium.dll
Required for PDF viewer functionality. The build process attempts to stage it automatically:
- **Automatic staging locations**:
  - `.vendor/pdfium.dll`
  - `.vendor/pdfium/pdfium.dll`
  - `$env:PDFIUM_DYNAMIC_LIB_PATH/pdfium.dll`
- **Manual placement**: Copy pdfium.dll next to the executable if automatic staging fails
- **Security verification**: Hash verification enforced in release builds

### Audio Runtime Notes
- Audio-only files are routed to the standalone mpv player with real-time waveform visualization
- Explicit fast-path audio extension handling includes: `mp3`, `wav`, `ogg`, `wma`, `aac`, `m4a`, `ape`, `mid`, `flac`, `alac`, `opus`, `aiff`, `weba`
- Audio metadata extraction reads duration, codec, bitrate, channels, sample rate, artist, album, track title, genre, and year
- Codec fallback sniffing recognizes: `AAC`, `MP3`, `FLAC`, `Opus`, `Vorbis`, `AC-3`, `E-AC-3`, `ALAC`, `PCM`, `WMA`, and `DTS`

**Section sources**
- [docs/02_build_run_debug.md:17-21](file://docs/02_build_run_debug.md#L17-L21)
- [docs/02_build_run_debug.md:37-47](file://docs/02_build_run_debug.md#L37-L47)
- [docs/02_build_run_debug.md:49-54](file://docs/02_build_run_debug.md#L49-L54)
- [build.rs:77-138](file://build.rs#L77-L138)

## Execution Modes

### Main Application
Normal operation with borderless window and integrated preview functionality.

### Standalone Viewers
Separate processes from the same executable using mode flags:
- **Image viewer**: `cargo run -- --image-viewer "<path>"`
- **PDF viewer**: `cargo run -- --pdf-viewer "<path>"`
- **Text viewer**: `cargo run -- --text-viewer "<path>"`
- **Video player**: `cargo run -- --video-player "<path>" [--position <seconds>] [--volume <0.0..1.0>]`

### Viewer Runtime
Uses a lightweight Glow renderer and reads theme/language preferences from a read-only SQLite query. The viewers run as separate processes spawned from the same binary as the main file manager.

**Section sources**
- [src/main.rs:143-215](file://src/main.rs#L143-L215)
- [src/viewer_runtime.rs:1-86](file://src/viewer_runtime.rs#L1-L86)

## Global Search Service

### Service Architecture
The search service (`mtt-search-service`) runs as a Windows Service with hybrid per-volume indexing:
- **NTFS/ReFs**: USN Journal (full MFT scan on first run + incremental loop)
- **Non-USN (exFAT/FAT32/FUSE/CryptoFS)**: Full-tree scan with SQLite cache + periodic re-scan

### Service Management
```powershell
# Build only the service binary
cargo build --release -p mtt-search-service

# Binary output
.\target\release\mtt-search-service.exe

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

### Non-USN Update Cadence
- **30s** for virtual filesystems (`fuse`, `cryptofs`, `dokan`, `winfsp`)
- **120s** for physical volumes without USN (e.g., exFAT/FAT32)

### IPC Hardening (Optional)
To reduce status metadata exposure in restricted environments:
```powershell
$env:MTT_SEARCH_REDACT_STATUS_METRICS = "1"
sc.exe stop MTTFileManagerSearch
sc.exe start MTTFileManagerSearch
```

**Section sources**
- [docs/02_build_run_debug.md:132-182](file://docs/02_build_run_debug.md#L132-L182)
- [crates/mtt-search-service/src/main.rs:129-156](file://crates/mtt-search-service/src/main.rs#L129-L156)

## Logging and Diagnostics

### Release Build Logging
Release builds of `mtt-file-manager.exe` use the Windows GUI subsystem, so launching the executable directly does not open an extra console window. For diagnostics, run it explicitly from PowerShell so stdout/stderr can be captured.

### Method 1: PowerShell Script (Recommended)
```powershell
.\run_with_logs.ps1
# Logs are saved to: debug_metadata.log
```

### Method 1B: Dedicated Diagnostic Console Window
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

### Environment Variables
```powershell
$env:RUST_BACKTRACE=1                    # Enable backtraces on panic
$env:RUST_BACKTRACE="full"               # Full backtraces
$env:RUST_LOG="debug"                    # Debug logging
$env:RUST_LOG="mtt_file_manager=debug"   # Module-specific logging
$env:CARGO_INCREMENTAL=1                 # Incremental compilation
```

**Section sources**
- [docs/02_build_run_debug.md:225-267](file://docs/02_build_run_debug.md#L225-L267)
- [run_with_logs.ps1:1-12](file://run_with_logs.ps1#L1-L12)
- [open_diagnostic_console.cmd:1-6](file://open_diagnostic_console.cmd#L1-L6)
- [docs/02_build_run_debug.md:393-401](file://docs/02_build_run_debug.md#L393-L401)

## Troubleshooting Guide

### Common Issues and Resolutions

#### libmpv-2.dll not found
```powershell
Copy-Item "path\to\libmpv-2.dll" -Destination ".\target\release\"
# Or add to PATH
$env:PATH += ";C:\Path\To\libmpv"
```

#### pdfium.dll not found
```powershell
# Option 1: provide it via the environment variable before building
$env:PDFIUM_DYNAMIC_LIB_PATH = "C:\Path\To\Pdfium"
cargo build --release --workspace

# Option 2: copy it manually next to the executable
Copy-Item "C:\Path\To\pdfium.dll" -Destination ".\target\release\"
```

#### Renderer initialization or blank startup window
If the default `wgpu` backend selection fails on a specific machine, force the OpenGL/ANGLE compatibility path for a diagnostic run of the **main file-manager window**:
```powershell
$env:WGPU_BACKEND = "opengl"
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "gpu-debug.log"
```

Notes:
- This override only affects the current PowerShell session
- If the app starts with `opengl`, the machine likely cannot initialize the preferred native backend reliably
- Startup logs include the selected adapter and backend, which helps confirm what `wgpu` actually used
- The standalone image/PDF/text viewers use `glow`, so this diagnostic is only relevant for the main window

Clear the override after testing:
```powershell
Remove-Item Env:WGPU_BACKEND
```

#### Slow Build
```bash
cargo build --release -j 8    # Parallel compilation
```

#### Windows API Compilation Error
Ensure Windows SDK is installed via Visual Studio Installer:
- MSVC v143
- Windows 10/11 SDK

### Verification Steps
- Use installer validation to confirm required files and hashes
- Run security verification suite for targeted tests
- Use diagnostic console scripts to capture logs

**Section sources**
- [docs/02_build_run_debug.md:321-369](file://docs/02_build_run_debug.md#L321-L369)
- [installer/build_installer.ps1:82-109](file://installer/build_installer.ps1#L82-L109)

## Performance Considerations

### Build Optimization
- **Release builds** enable aggressive optimization and LTO for best performance
- **Single codegen unit** configuration for optimal optimization
- **Maximum optimization level** (opt-level = 3) for production builds

### Rendering Performance
- **GPU backend selection** prefers HighPerformance; viewer runtime uses a lightweight renderer to minimize memory footprint
- **Per-Monitor V2 DPI awareness** avoids DWM bitmap-scaling overhead and ensures crisp rendering on high-DPI displays
- **Modern Windows compatibility** settings improve rendering performance and reliability

### Memory Management
- **Standalone viewers** use `glow` renderer instead of `wgpu` to avoid large DX12 baseline that dominates RSS
- **Lightweight SQLite connections** for preference reading in viewers
- **Disabled optional GL buffers** (`depth_buffer`, `stencil_buffer`, `multisampling`) which viewers never use

**Section sources**
- [docs/02_build_run_debug.md:113-131](file://docs/02_build_run_debug.md#L113-L131)
- [docs/05_dependencies_stack.md:179-184](file://docs/05_dependencies_stack.md#L179-L184)
- [app.manifest:10-17](file://app.manifest#L10-L17)

## Security and Verification

### Security Verification Suite
Security verification runs targeted tests for the search service and protocol. The installer validates third-party DLL integrity before packaging.

### Installer Security Measures
- **Hash verification** for critical DLLs (`pdfium.dll`, `libmpv-2.dll`)
- **Third-party component validation** during installation
- **Security hardening** for service execution as LocalSystem

**Section sources**
- [security_verify.ps1:1-24](file://security_verify.ps1#L1-L24)
- [installer/build_installer.ps1:82-109](file://installer/build_installer.ps1#L82-L109)

## Appendices

### Appendix A: Build and Run Commands
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

**Section sources**
- [docs/02_build_run_debug.md:370-391](file://docs/02_build_run_debug.md#L370-L391)

### Appendix B: Runtime Dependency Locations
- **libmpv-2.dll**: Next to executable or in PATH
- **pdfium.dll**: Vendor directory or `PDFIUM_DYNAMIC_LIB_PATH`; staged automatically by build.rs
- **Portable mpv configuration**: `mpv_ui/portable_config/` directory

**Section sources**
- [docs/02_build_run_debug.md:17-21](file://docs/02_build_run_debug.md#L17-L21)
- [docs/02_build_run_debug.md:37-47](file://docs/02_build_run_debug.md#L37-L47)
- [build.rs:77-138](file://build.rs#L77-L138)

### Appendix C: Installer Artifacts and Behavior
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

**Section sources**
- [docs/02_build_run_debug.md:183-224](file://docs/02_build_run_debug.md#L183-L224)
- [installer/build_installer.ps1:192-214](file://installer/build_installer.ps1#L192-L214)

### Appendix D: VS Code Settings
```json
{
    "rust-analyzer.cargo.features": ["notify-watcher"],
    "rust-analyzer.checkOnSave.command": "clippy",
    "rust-analyzer.cargo.buildScripts.enable": true
}
```

**Section sources**
- [docs/02_build_run_debug.md:403-411](file://docs/02_build_run_debug.md#L403-L411)