# 📚 Stack Técnico Completo - MTT File Manager

## Core Technologies

### 🦀 Rust (Edition 2021)

**Por que Rust?**
- **Zero-cost abstractions**: Performance nativa sem overhead
- **Memory safety**: Sem garbage collector, sem memory leaks
- **Concurrency sem data races**: Sistema de ownership garante thread-safety
- **Executável tiny**: 4-6 MB final (vs 60-100 MB Electron)

**Perfil de Release**:
```toml
[profile.release]
opt-level = 3        # Máxima otimização
lto = true           # Link-Time Optimization
codegen-units = 1    # Melhor otimização cross-crate
```

---

## UI Framework

### eframe 0.31 (egui)

**Categoria**: Immediate Mode GUI  
**Renderização**: OpenGL 3.3+ / WebGL2  
**Paradigma**: Retained mode rendering com declarative API

**Características**:
- ✅ **Hot reload friendly**: UI se reconstrói a cada frame
- ✅ **GPU accelerated**: Texturas em VRAM
- ✅ **Cross-platform**: Windows, Linux, macOS, Web (WASM)
- ✅ **Zero unsafe code**: API 100% safe Rust
- ⚠️ **Não é nativo**: Desenha custom controls (não usa Win32 controls)

**Principais Tipos Usados**:
```rust
eframe::App              // Trait principal da aplicação
egui::Context            // Estado global da UI
egui::TextureHandle      // Handle para texturas GPU
egui::ColorImage         // Buffer RGBA para texturas
egui::Grid               // Layout em grid
egui::ScrollArea         // Scroll virtualizado
```

**Renderização**:
```
CPU (Rust) → Tessellation → GPU Buffers → OpenGL → Display
         ↑                              ↑
    60 FPS update                   VRAM textures
```

---

## Paralelismo e Concorrência

### rayon 1.10

**Categoria**: Data parallelism library  
**Thread Pool**: Automático (baseado em CPU cores)

**Uso no Projeto**:
- ❌ **Ainda não usado diretamente** (apenas como dependência transitiva)
- 🎯 **Candidato**: Processamento paralelo de thumbnails em batch

**Exemplo de uso futuro**:
```rust
use rayon::prelude::*;

paths.par_iter()
    .map(|path| extract_windows_thumbnail(path))
    .collect()
```

### std::sync::mpsc

**Categoria**: Multi-producer, single-consumer channel  
**Thread-safety**: Sim (Rust garantias)

**Uso Atual**:
```rust
let (sender, receiver) = mpsc::channel();

// Worker threads → UI thread
sender.send(ThumbnailData { ... });

// UI thread (non-blocking)
while let Ok(data) = receiver.try_recv() {
    // Process thumbnails
}
```

---

## Filesystem

### walkdir 2.5

**Categoria**: Recursive directory iterator  
**Performance**: ~10,000 files/segundo

**Características**:
- ✅ Cross-platform (Windows, Unix)
- ✅ Segue symlinks opcionalmente
- ✅ Controle de profundidade (`max_depth`)
- ✅ Filtragem custom via iterators

**Uso no Projeto**:
```rust
WalkDir::new(&path)
    .max_depth(1)              // Não recursivo
    .into_iter()
    .filter_map(|e| e.ok())    // Ignora erros de I/O
    .filter(|e| /* custom filters */)
```

### notify 6.1.1

**Categoria**: Cross-platform filesystem notification library  
**Backend Windows**: ReadDirectoryChangesW (Async I/O)

**Uso no Projeto**:
- ✅ **Auto-Refresh**: Monitora a pasta atual para mudanças.
- ✅ **Debounce**: Evita reloads excessivos em operações rápidas de I/O.
- ✅ **Comunicação**: Thread do Watcher → Canal MPSC → Main Thread.

---

### windows 0.58 (Microsoft Official Crate)

**Categoria**: Safe Rust bindings para Win32 APIs  
**Cobertura**: ~30,000 APIs do Windows

**APIs Utilizadas**:

#### 1. Shell (UI Integration)
```rust
Win32::UI::Shell::{
    SHCreateItemFromParsingName,  // Path → IShellItem
    IShellItemImageFactory,        // Interface para thumbnails
    ShellExecuteW,                 // Abre arquivos com app padrão
}
```

#### 2. COM (Component Object Model)
```rust
Win32::System::Com::{
    CoInitializeEx,        // Inicializa COM em thread
    CoUninitialize,        // Cleanup
    COINIT_MULTITHREADED,  // Modo apartamento
}
```

#### 3. GDI (Graphics Device Interface)
```rust
Win32::Graphics::Gdi::{
    GetObjectW,        // Info sobre HBITMAP
    GetDC,             // Device Context
    ReleaseDC,         // Cleanup DC
    GetDIBits,         // HBITMAP → raw pixels
    DeleteObject,      // Libera HBITMAP
}
```

#### 4. File System
```rust
Win32::Storage::FileSystem::{
    GetLogicalDriveStringsW,    // Lista drives (C:\, D:\)
    GetFileAttributesW,         // Atributos (hidden, system)
    FILE_ATTRIBUTE_HIDDEN,
    FILE_ATTRIBUTE_SYSTEM,
}
```

#### 5. Foundation
```rust
Win32::Foundation::{
    HBITMAP,       // Handle para bitmap
    SIZE,          // Estrutura width/height
    RECT,          // Retângulo
}
```

**Por que Windows Crate?**
- ✅ **Bindings oficiais da Microsoft**
- ✅ **Type-safe**: Erros em compile-time
- ✅ **Zero overhead**: Thin wrappers
- ✅ **Auto-generated**: Sempre atualizado
- ⚠️ **Vendor lock-in**: Aplicação 100% Windows-only

---

## Utilities

### rfd 0.15 (Rust File Dialog)

**Categoria**: Native file picker dialogs  
**Backend Windows**: IFileDialog (COM)

**Uso no Projeto**:
```rust
// Futuro: Botão "Escolher Pasta"
rfd::FileDialog::new()
    .set_directory("/")
    .pick_folder()
```

### lru 0.12

**Categoria**: LRU Cache implementation  
**Complexidade**: O(1) para get/put

**Uso no Projeto**:
```rust
LruCache<PathBuf, egui::TextureHandle>
    .put(path, texture);  // Insere ou atualiza
    .get(&path);          // Retorna Option<&Texture>
    // Eviction automática quando atinge CACHE_SIZE
```

**Características**:
- ✅ Thread-safe com `Mutex<LruCache>`
- ✅ Determinístico: Sempre remove least-recently-used
- ⚠️ Não persiste entre sessões

---

## Dependências Transitivas Importantes

### Gráficos e Rendering

| Crate | Propósito |
|-------|-----------|
| `glutin` | OpenGL context creation |
| `glutin_egl_sys` | EGL bindings (Windows) |
| `glutin_wgl_sys` | WGL bindings (Windows) |
| `winit` | Cross-platform window creation |
| `ab_glyph` | Font rasterization |

### Serialização (egui internal)

| Crate | Propósito |
|-------|-----------|
| `serde` | Serialization framework |
| `serde_derive` | Macros para `#[derive(Serialize)]` |

### Compressão (PNG decoding)

| Crate | Propósito |
|-------|-----------|
| `png` | PNG encoder/decoder |
| `flate2` | zlib compression |
| `crc32fast` | CRC32 checksum |

---

## Ferramentas de Build

### Cargo (Rust Package Manager)

**Comandos Principais**:
```powershell
cargo build --release     # Build otimizado
cargo run                 # Debug mode
cargo clean               # Limpa target/
cargo check               # Verifica sem compilar
cargo clippy              # Linter
cargo fmt                 # Code formatter
```

**Artifacts**:
```
target/
├── debug/
│   └── mtt-file-manager.exe    (~30 MB, sem otimizações)
└── release/
    └── mtt-file-manager.exe    (~4 MB, otimizado + stripped)
```

---

## Bibliotecas Ausentes (Candidatas)

### 🎯 Sugestões para Adicionar

| Biblioteca | Propósito | Prioridade |
|-----------|-----------|-----------|
| `tracing` | Logging estruturado | 🔥 Alta |
| `anyhow` | Error handling ergonômico | 🔥 Alta |
| `serde_json` | Config persistence | Média |
| `tokio` | Async runtime (future) | Baixa |
| `image` | Decode direto (fallback) | Baixa |

---

## Comparação com Alternativas

### vs Electron (Node.js + Chromium)

| Métrica | MTT (Rust) | Electron |
|---------|-----------|----------|
| **Executável** | 4-6 MB | 60-150 MB |
| **RAM idle** | ~50 MB | ~200-400 MB |
| **Startup time** | <500ms | 1-3s |
| **Native feel** | ⚠️ Semi | ❌ Não |
| **Dev experience** | ⚠️ Steep | ✅ Fácil |

### vs Tauri (Rust + WebView)

| Métrica | MTT (egui) | Tauri |
|---------|-----------|--------|
| **Executável** | 4-6 MB | 8-12 MB |
| **UI tech** | OpenGL | HTML/CSS/JS |
| **Performance** | ✅ 60+ FPS | ⚠️ Depende do WebView |
| **Windows API** | ✅ Direto | ⚠️ Via bridges |

---

## Versioning e Updates

**Rust Toolchain**:
```
rustc 1.75+ (stable)
cargo 1.75+
```

**Dependências Lock**:
- `Cargo.lock` commitado no repo
- Garante builds reproduzíveis
- Atualização via `cargo update`

---

## Próximos Passos Tecnológicos

Ver [ROADMAP_TECNICO.md](ROADMAP_TECNICO.md) para:
- Migração para arquitetura modular
- Adição de logging (`tracing`)
- Testes unitários com `mockall`
- CI/CD com GitHub Actions
