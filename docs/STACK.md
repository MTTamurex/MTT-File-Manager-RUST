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

#### 6. Media Foundation (Video Metadata)
```rust
Win32::Media::MediaFoundation::{
    MFStartup, MFShutdown,           // Lifecycle management
    MFCreateSourceReaderFromURL,      // Open video files
    IMFSourceReader,                  // Read video streams
    IMFMediaType,                     // Stream format info
    MF_MT_FRAME_SIZE,                 // Video resolution
    MF_MT_FRAME_RATE,                 // Video framerate
    MF_MT_SUBTYPE,                    // Codec GUID
    MF_PD_DURATION,                   // Video duration
}

Win32::System::Registry::{          // Codec name resolution (NEW 2026-01-07)
    RegOpenKeyExW, RegGetValueW,     // Query CLSID friendly names
    HKEY_LOCAL_MACHINE,              // Root key for system codecs
}
```

**Codec Resolution Strategy** (Implements .cursorrules §7):
1. **Microsoft SDK Database**: Official codec GUIDs from Windows SDK headers (mfapi.h, wmcodecdsp.h, mmreg.h) - resolves codecs not registered in system
2. **LRU Cache**: 128 entries, thread-safe `Mutex<LruCache>`
3. **Windows Registry**: `HKLM\SOFTWARE\Classes\CLSID\{GUID}\FriendlyName`
4. **Media Foundation Transform**: MFTEnumEx with input/output type filters across decoder/encoder categories
5. **3-Layer Architecture**: SDK DB → Registry → MFT ensures maximum codec coverage

**Supported GUID Formats**:
- Full GUID with braces: `{A7FB87AF-0000-0010-8000-00AA00389B71}` → `Dolby Digital Plus (EAC-3)`
- Partial hex (8 digits): `E06D802C` → auto-expands to full GUID → `Dolby Digital Plus (DD+)`
- GUID without braces: `A7FB87AF-0000-0010-8000-00AA00389B71` → `Dolby Digital Plus (EAC-3)`

**Microsoft SDK Codec Database** (official GUIDs, not arbitrary hardcoding):
- **Dolby**: EAC-3 (A7FB87AF), DD+ (E06D802C), AC-4 (0000240C)
- **Windows Media**: WMA9 Lossless (00000162), WMA9 Pro (00000163), WMA10 Pro (00000166), WMAv2 (00000161)
- **MPEG**: AAC (00006C75), MP3 (0000706D/00000055), MP2 (00000050)
- **DivX**: DivX Audio (00004143)
- **Video**: H.264, H.265, MPEG-4, VC-1, VP8, VP9, AV1 (via MFT enumeration)

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

### rusqlite 0.32

**Categoria**: SQLite bindings for Rust  
**Modo**: Bundled (sem dependência de DLL externa)

**Uso no Projeto**:
- ✅ **Persistência de Thumbnails**: Armazenamento binário (BLOB) de WebP lossy Q60.
- ✅ **Tamanhos Adaptativos**: Imagens >512px downscaled, vídeos preservados em 256px.
- ✅ **Concurrency**: PRAGMA journal_mode = WAL permite leituras e escritas seguras entre workers.
- ✅ **Performance**: SQL query otimizada com indexação por hash do path.

### image 0.25

**Categoria**: Image processing library  
**Codecs**: WebP, JPEG, PNG, BMP, GIF, TIFF

**Uso no Projeto**:
- ✅ **Stage 1 Extraction**: Decodificação rápida de imagens padrão RGB.
- ✅ **Cache Decoding**: Conversão de WebP (cache SQLite) para RGBA (egui).
- ✅ **Resize Adaptativo**: Downscale inteligente via `Lanczos3` (preserva ≤512px, reduz >512px).

### webp 0.3

**Categoria**: WebP encoding/decoding  
**Backend**: libwebp (Google)

**Uso no Projeto**:
- ✅ **Lossy Compression**: Encode WebP com quality 60 para thumbnails.
- ✅ **Tamanho Otimizado**: ~30KB por thumbnail em 512px (balanço qualidade/espaço).
- ✅ **HiDPI Support**: Mantém qualidade visual em displays 2x (200% DPI).

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

### natord 1.0

**Categoria**: Natural/Smart Sorting  
**Complexidade**: O(n) comparação com parsing de números

**Uso no Projeto**:
```rust
// Ordena "File1, File2, File10" em vez de "File1, File10, File2"
natord::compare(&a.name.to_lowercase(), &b.name.to_lowercase())
```

**Características**:
- ✅ Leve (~50 linhas de código, MIT)
- ✅ Zero allocations durante comparação
- ✅ Trata números embarcados em strings corretamente

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

## Bibliotecas Implementadas Recentemente (2026-01-01)

### ✅ Novas Dependências em Uso

| Biblioteca | Propósito | Status |
|-----------|-----------|--------|
| **`rusqlite`** | Cache persistente em SQLite | ✅ Implementado |
| **`image`** | Redimensionamento e WebP | ✅ Implementado |
| **`tracing`** | Logging estruturado | 🚧 Planejado |

### ✅ Módulos Implementados (Sem Dependências Externas)

| Módulo | Localização | Status |
|--------|------------|--------|
| **Icon Loader Assíncrono** | `src/ui/icon_loader.rs` | ✅ Implementado |
| **CacheManager Unificado** | `src/ui/cache.rs` | ✅ Implementado |
| **Notification System** | `src/application/notification.rs` | ✅ Implementado |
| **Thumbnail Worker** | `src/workers/thumbnail_worker.rs` | ✅ Implementado |
| **Codec Registry (Dynamic)** | `src/infrastructure/windows/codec_registry.rs` | ✅ Implementado (2026-01-07) |
| **Windows API Wrappers** | `src/infrastructure/windows/` | ✅ Implementado (8 módulos) |
| **Windows Clipboard (CF_HDROP)** | `src/infrastructure/windows_clipboard.rs` | ✅ Implementado (2026-01-04) |

### clipboard-win 5.4

**Categoria**: Windows Clipboard API bindings  
**Formato**: CF_HDROP (file list)

**Uso no Projeto**:
- ✅ **Ctrl+C**: Copia arquivo para clipboard Windows (CF_HDROP)
- ✅ **Ctrl+X**: Recorta arquivo (CF_HDROP + Preferred DropEffect = MOVE)
- ✅ **Ctrl+V**: Lê arquivos do clipboard Windows
- ✅ **Menu de Contexto Nativo**: "Colar" aparece automaticamente no menu nativo
- ✅ **Cross-App**: Copiar no Explorer → Colar no MTT (e vice-versa)

**Exemplo**:
```rust
use clipboard_win::{formats, Clipboard, Setter, Getter};

// Copiar arquivo para clipboard
let _clip = Clipboard::new_attempts(10)?;
formats::FileList.write_clipboard(&["C:\\path\\to\\file.txt"])?;

// Ler arquivos do clipboard
let mut files: Vec<String> = Vec::new();
formats::FileList.read_clipboard(&mut files)?;
```

### 🎯 Próximas Dependências Sugeridas

| Biblioteca | Propósito | Prioridade | Justificativa |
|-----------|-----------|-----------|---------------|
| `tracing` + `tracing-subscriber` | Logging estruturado | 🔥 Alta | Debug em produção, monitoramento |
| `anyhow` + `thiserror` | Error handling | 🔥 Alta | Tratamento robusto de erros |
| `serde` + `serde_json` | Config persistence | ⚠️ Média | Preferências do usuário |
| `mockall` | Testes unitários | ⚠️ Média | Mocking de APIs Windows |
| `proptest` | Property-based testing | 🟢 Baixa | Testes mais robustos |

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
