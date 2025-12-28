# MTT File Manager - AI Coding Agent Instructions

## Project Overview

A high-performance Windows-native file manager in Rust focused on ultra-fast image/video thumbnail rendering using native Win32 APIs. Single-file monolith (~1575 lines in [src/main.rs](../src/main.rs)) with immediate-mode GUI (egui).

**Key Metrics**: 4-6 MB binary | Windows 10+ only | Zero external dependencies

## Critical Architecture Patterns

### 1. Documentation-First Development

**MANDATORY**: Before ANY code change, read relevant `docs/` files and update them immediately after:

| Change Type | Update File |
|-------------|-------------|
| Add/remove dependency | [docs/STACK.md](../docs/STACK.md) |
| Windows API (unsafe blocks) | [docs/SEGURANCA_WINDOWS.md](../docs/SEGURANCA_WINDOWS.md) |
| Architecture/data flow | [docs/ARQUITETURA.md](../docs/ARQUITETURA.md) |
| Technical debt | [docs/ROADMAP_TECNICO.md](../docs/ROADMAP_TECNICO.md) |

Commit format: `feat: X | docs: atualiza Y`

### 2. Async/Parallel Architecture

**Pattern**: UI thread never blocks. Background loading via `mpsc::channel`:

```rust
// Worker threads → UI thread
std::thread::spawn(move || {
    let thumbnail = extract_windows_thumbnail(&path);
    sender.send(ThumbnailData { ... }).ok();
});

// UI thread (non-blocking)
while let Ok(data) = self.image_receiver.try_recv() {
    let texture = ctx.load_texture(...);
    self.texture_cache.put(data.path, texture);
}
```

**Concurrency Limits**: MAX_CONCURRENT_LOADS = 30 (prevents resource exhaustion)

### 3. Memory Management (Critical for Desktop Apps)

- **LRU Cache**: `CACHE_SIZE = 200` thumbnails (~50-100MB VRAM max)
- **Lazy Loading**: Only load thumbnails in viewport (virtualização)
- **Viewport Culling**: Calculate `min_row/max_row`, skip offscreen items
- **RAII for Windows Resources**: Always pair `GetImage` with `DeleteObject`

Example leak prevention:
```rust
unsafe {
    let hicon = shfi.hIcon;
    let result = hicon_to_rgba(hicon);
    DestroyIcon(hicon);  // MUST cleanup!
    result
}
```

### 4. Windows Native Integration

**Thumbnail Loading**: Uses `IShellItemImageFactory` (same as Windows Explorer):
```rust
unsafe {
    CoInitializeEx(None, COINIT_MULTITHREADED)?;
    let item: IShellItem = SHCreateItemFromParsingName(...)?;
    let factory: IShellItemImageFactory = item.cast()?;
    let hbitmap = factory.GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF_THUMBNAILONLY)?;
    // Convert BGRA → RGBA for egui
    CoUninitialize();
}
```

**Icon Caching**: Three-tier system in [main.rs](../src/main.rs#L180-L185):
- `icon_cache`: extension → texture (LRU 100 items)
- `folder_icon_texture`: Shared folder icon
- `drive_icon_cache`: drive path → icon

## Development Workflows

### Build Commands
```powershell
# Debug (30 MB, fast compile)
cargo run

# Release (4-6 MB, LTO enabled)
cargo build --release
.\target\release\mtt-file-manager.exe

# Format & Lint (REQUIRED before commit)
cargo fmt --all
cargo clippy -- -D warnings
```

### No Test Suite Yet
**Current State**: Zero unit tests (see [docs/ROADMAP_TECNICO.md](../docs/ROADMAP_TECNICO.md#2-zero-testes-unitários))  
**When adding tests**: Use `cargo test`, consider `mockall` for Win32 API mocking

## Code Conventions & Anti-Patterns

### ✅ Do
- Use `Result<T, E>` instead of `.unwrap()` (zero panics in production)
- Sanitize paths before filesystem operations (see [docs/SEGURANCA_WINDOWS.md](../docs/SEGURANCA_WINDOWS.md#1-path-traversal))
- Document `unsafe` blocks with `// SAFETY:` comments
- Pre-check in whitelist before `ShellExecuteW` (safe extensions only)
- Use `LruCache` for bounded collections

### ❌ Don't
- Add dependencies without updating [docs/STACK.md](../docs/STACK.md) (whitelist: eframe, rayon, walkdir, rfd, lru, windows)
- Use placeholders like `...existing code...` in edits
- Load all items eagerly (use viewport-based lazy loading)
- Create unbounded threads (respect MAX_CONCURRENT_LOADS)
- Ignore channel send errors (`let _ = sender.send(...)`)

### Allowed Dependencies
```toml
eframe = "0.31"      # UI framework (egui)
rayon = "1.10"       # Thread pool (currently unused)
walkdir = "2.5"      # Filesystem iteration
rfd = "0.15"         # Native folder picker
lru = "0.12"         # LRU cache
windows = "0.58"     # Win32 APIs
```

## Known Technical Debt (from Roadmap)

1. **Monolithic main.rs** (1575 lines): Needs modularization into `ui/`, `domain/`, `infrastructure/`
2. **No tests**: Critical for preventing regressions
3. **Path traversal vulnerability**: Missing `sanitize_path()` validation
4. **Command injection**: Weak extension filtering in `open_with_shell()`

## Key Files to Reference

- [src/main.rs](../src/main.rs): Monolithic app (all code here currently)
- [docs/ARQUITETURA.md](../docs/ARQUITETURA.md): Mermaid diagrams, data flow
- [docs/SEGURANCA_WINDOWS.md](../docs/SEGURANCA_WINDOWS.md): Security vectors & mitigations
- [Cargo.toml](../Cargo.toml): Dependencies + release profile (LTO, opt-level 3)

## egui-Specific Patterns

**Immediate Mode**: UI rebuilt every frame (60 FPS). State in `ImageViewerApp` struct.

**Grid Rendering** (custom, not `egui::Grid`):
```rust
let visible_rect = ui.clip_rect();
let min_row = (scroll_offset / row_height).floor() as usize;
let max_row = ((scroll_offset + visible_rect.height()) / row_height).ceil() as usize;

for row in min_row..=max_row {
    for col in 0..cols_per_row {
        let pos = egui::pos2(x_base + col_offset, y_base + row_offset);
        // Render only if ui.is_rect_visible(item_rect)
    }
}
```

**Texture Lifecycle**: `ctx.load_texture()` → store `TextureHandle` → auto-cleanup on eviction

## Security Checklist (Before Shipping Features)

- [ ] All paths canonicalized with `std::fs::canonicalize`
- [ ] Forbidden paths blocked (System32, SysWOW64, WindowsApps)
- [ ] Extension whitelist enforced before `ShellExecuteW`
- [ ] All `unsafe` blocks documented in [docs/SEGURANCA_WINDOWS.md](../docs/SEGURANCA_WINDOWS.md)
- [ ] No `.unwrap()` or `.expect()` in production paths
- [ ] Windows resource cleanup (HBITMAP, HICON, COM)
