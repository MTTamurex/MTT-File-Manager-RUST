# Documentation Index — MTT File Manager

## Documents

### [01_overview.md](01_overview.md)
**Overview** — Project introduction, key features, high-level architecture, technology stack, system requirements.

### [02_build_run_debug.md](02_build_run_debug.md)
**Build, Run & Debug** — Prerequisites, build instructions, running with logs, debug/profiling, troubleshooting.

### [03_architecture.md](03_architecture.md)
**Architecture** — Layered architecture, workspace crates, component responsibilities, communication patterns, lifecycle.

### [04_module_map.md](04_module_map.md)
**Module Map** — Complete directory tree with module descriptions, data flow, module rules.

### [05_dependencies_stack.md](05_dependencies_stack.md)
**Dependency Stack** — All Cargo dependencies with versions, Windows API features, runtime dependencies, build profiles.

### [06_key_flows.md](06_key_flows.md)
**Key Flows** — Step-by-step documentation of major application flows: navigation, preview, file operations, thumbnail pipeline, global search, image/video/PDF viewers.

### [07_storage_config.md](07_storage_config.md)
**Storage & Configuration** — Data locations, SQLite schemas (app + search service), cache structure, preferences, i18n.

### [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md)
**Logging, Errors & Telemetry** — Log categories, capture methods, AppError type, debugging techniques.

### [09_performance_optimizations.md](09_performance_optimizations.md)
**Performance Optimizations** — Drive-wide watcher, NtQueryDirectoryFile, smart DELETE handling, thumbnail pipeline, folder cover composition, I/O priority, caching strategies.

## Quick Reference

### New to the project?
1. Start with [01_overview.md](01_overview.md) for a high-level understanding
2. Read [02_build_run_debug.md](02_build_run_debug.md) to build and run
3. Review [03_architecture.md](03_architecture.md) for the architecture
4. Browse [04_module_map.md](04_module_map.md) to find specific code

### Debugging an issue?
1. Check [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) for log capture methods
2. Review [06_key_flows.md](06_key_flows.md) for the relevant flow

### Understanding dependencies?
→ [05_dependencies_stack.md](05_dependencies_stack.md)

### Performance questions?
→ [09_performance_optimizations.md](09_performance_optimizations.md)

### Where is data stored?
→ [07_storage_config.md](07_storage_config.md)

## Project Structure

```
MTT-File-Manager-RUST/
├── src/                    # Main application
│   ├── app/                # State & initialization
│   ├── application/        # Business logic services
│   ├── domain/             # Core data models
│   ├── infrastructure/     # System integration & Windows APIs
│   ├── ui/                 # User interface (eframe/egui)
│   ├── workers/            # Background worker threads
│   ├── image_viewer/       # Dedicated image viewer (separate process)
│   ├── video_player/       # Standalone video player (separate process)
│   ├── pdf_viewer/         # Native PDF viewer (separate process)
│   └── tabs/               # Tab management
├── crates/
│   ├── mtt-search-protocol/  # IPC types (bincode)
│   └── mtt-search-service/   # Windows Service for file indexing
├── locales/                # i18n translation files (en, pt-BR)
├── DOCs/                   # This documentation
└── benches/                # Benchmarks
```

## Entry Points

| Binary | Entry | Purpose |
|--------|-------|---------|
| `mtt-file-manager` | `src/main.rs` | Main file manager GUI |
| `mtt-file-manager --image-viewer <path>` | `src/image_viewer/` | Dedicated image viewer |
| `mtt-file-manager --video-player <path>` | `src/video_player/` | Standalone video player |
| `mtt-file-manager --pdf-viewer <path>` | `src/pdf_viewer/` | Native PDF viewer |
| `mtt-file-manager --set-volume-label <drive> <label>` | `src/main.rs` | Elevated helper for renaming drive volume labels |
| `mtt-search-service` | `crates/mtt-search-service/src/main.rs` | File indexing Windows Service |

## Useful Commands

```bash
cargo build --workspace           # Build everything
cargo run                         # Run in debug mode
cargo build --release --workspace # Release build
cargo bench                       # Run benchmarks
cargo clippy                      # Lint
cargo fmt                         # Format code
```

