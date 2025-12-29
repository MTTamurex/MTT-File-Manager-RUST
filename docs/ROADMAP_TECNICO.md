# 🗺️ Roadmap Técnico - MTT File Manager

## ✅ Features Implementadas (2025-12-28)

### Ícones Nativos do Windows
- **SHGetFileInfoW** para ícones de arquivos por extensão (SHGFI_USEFILEATTRIBUTES)
- **SHGetFileInfoW** para ícones de pastas (dummy path + FILE_ATTRIBUTE_DIRECTORY)
- **SHGetFileInfoW** para ícones reais de drives (path real, sem USEFILEATTRIBUTES)
- **GetVolumeInformationW** para labels de volumes (ex: "Sistema (C:)")
- **Cache LRU**: icon_cache, folder_icon_texture, drive_icon_cache

### Grid com Posicionamento Absoluto (Game Engine Style)
- Substituído `egui::Grid` por cálculo manual de coordenadas
- Altura de célula RÍGIDA: `thumbnail_size + 40px`
- Virtualização via `clip_rect()` e cálculo de `min_row/max_row`
- Culling estrito com `is_rect_visible()` antes de renderizar
- Rolagem butter-smooth sem jitter

### Sistema de Navegação Completo
- **navigation_history**: Vec<String> com histórico completo
- **history_index**: Posição atual na linha do tempo
- **path_input**: Barra de endereço editável
- Botões: Voltar (⬅), Avançar (➡), Subir (⬆)
- Botões desabilitados quando não aplicável

### Otimização de VRAM
- CACHE_SIZE reduzido de 500 → 200
- MAX_CONCURRENT_LOADS reduzido de 50 → 30
### Estabilidade e Gerenciamento de Ciclo de Vida (Anti-Leak)
- **Persistent Worker Pool**: Fila de 4 threads para evitar disk thrashing em HDDs externos.
- **Atomic Generational Validation**: Sistema de gerações que invalida tasks de thumbnails via `Arc<AtomicUsize>`.
- **Reactive Repaints**: Workers agora disparam `ctx.request_repaint()` imediatamente ao completar tarefas, garantindo fluidez sem depender de inputs do usuário.
- **CreationContext Initialization**: Refatorada inicialização para captar o contexto egui nativamente.
- **Native Rename (F2)**: Implementado renomeação via `SHFileOperationW` com suporte a Undo (Ctrl+Z).
- **Manual & Auto Refresh**: Sistema de recarga via F5 e monitoramento automático via `notify` crate com debounce de 500ms.

---

## Débitos Técnicos Identificados

### 🔴 Crítico

#### 1. Monolito de 675 Linhas

**Problema**: Todo código em `main.rs`, dificultando manutenção e testes.

**Impacto**:
- Difícil de testar unitariamente
- Conflitos em merges
- Onboarding de novos devs lento

**Solução Proposta**:
```
src/
├── main.rs (50 linhas)
├── lib.rs
├── ui/
│   ├── mod.rs
│   ├── app.rs (ImageViewerApp)
│   └── components/
│       ├── sidebar.rs
│       ├── grid.rs
│       └── item_slot.rs
├── domain/
│   ├── mod.rs
│   ├── filesystem.rs
│   └── thumbnail.rs
└── infrastructure/
    ├── mod.rs
    ├── windows_api.rs
    └── cache.rs
```

**Tempo Estimado**: 8 horas  
**Prioridade**: 🔥 Alta

---

#### 2. Zero Testes Unitários

**Problema**: Sem testes automatizados, regressões são fáceis.

**Solução**:
```rust
// tests/filesystem_test.rs
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sanitize_path() {
        assert!(sanitize_path("C:\\..\\Windows").is_err());
        assert!(sanitize_path("C:\\Users\\Public").is_ok());
    }
    
    #[test]
    fn test_safe_extensions() {
        assert!(is_safe_extension("jpg"));
        assert!(!is_safe_extension("exe"));
    }
}
```

**Ferramentas**:
- `cargo test` (built-in)
- `mockall` para mocking de Windows APIs
- `proptest` para property-based testing

**Tempo Estimado**: 16 horas (cobertura 60%)  
**Prioridade**: 🔥 Alta

---

#### 3. Ausência de Logging

**Problema**: Debug em produção é impossível.

**Solução**: Adicionar `tracing`
```rust
use tracing::{info, warn, error, debug};

#[instrument]
fn load_folder(&mut self) {
    info!("Loading folder: {}", self.current_path);
    
    match scan_directory(&self.current_path) {
        Ok(items) => {
            info!("Found {} items", items.len());
        }
        Err(e) => {
            error!("Failed to scan: {:?}", e);
        }
    }
}
```

**Output**:
```
[INFO  mtt_file_manager] Loading folder: C:\Users\Public\Pictures
[DEBUG mtt_file_manager] Found 127 files, 5 directories
[WARN  mtt_file_manager] Skipped 3 hidden files
```

**Dependências**:
```toml
[dependencies]
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

**Tempo Estimado**: 4 horas  
**Prioridade**: 🔥 Alta

---

### 🟡 Importante

#### 4. Error Handling Frágil

**Problema**: Muitos `.unwrap()`, `.expect()`, `let _ =` ignorando erros.

**Exemplos**:
```rust
// ❌ Código atual
let parent = Path::new(&self.current_path).parent().unwrap();

// ❌ Ignora erro silenciosamente
let _ = sender.send(thumbnail_data);

// ❌ Panic em produção
let texture = ctx.load_texture(...).expect("Failed to load");
```

**Solução**: Usar `anyhow` e `thiserror`
```rust
use anyhow::{Context, Result};
use thiserror::Error;

#[derive(Error, Debug)]
enum FileManagerError {
    #[error("Invalid path: {0}")]
    InvalidPath(String),
    
    #[error("IO error")]
    Io(#[from] std::io::Error),
    
    #[error("Thumbnail extraction failed")]
    ThumbnailError(#[from] ThumbnailError),
}

fn go_up_one_level(&mut self) -> Result<()> {
    let parent = Path::new(&self.current_path)
        .parent()
        .ok_or(FileManagerError::InvalidPath("No parent".into()))?;
    
    self.current_path = parent.to_string_lossy().to_string();
    self.load_folder()?;
    
    Ok(())
}
```

**Tempo Estimado**: 12 horas  
**Prioridade**: ⚠️ Média-Alta

---

#### 5. Falta de Configuração Persistente

**Problema**: Usuário perde preferências ao fechar app.

**Features Desejadas**:
- Última pasta aberta
- Tamanho de zoom
- Window size/position
- Theme (claro/escuro)

**Solução**:
```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct AppConfig {
    last_path: String,
    thumbnail_size: f32,
    window_width: f32,
    window_height: f32,
}

impl AppConfig {
    fn load() -> Result<Self> {
        let config_path = dirs::config_dir()
            .unwrap()
            .join("MTT File Manager")
            .join("config.json");
        
        let content = std::fs::read_to_string(config_path)?;
        Ok(serde_json::from_str(&content)?)
    }
    
    fn save(&self) -> Result<()> {
        // ...
    }
}
```

**Dependências**:
```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
dirs = "5.0"
```

**Tempo Estimado**: 6 horas  
**Prioridade**: ⚠️ Média

---

#### 6. Performance - Batch Thumbnail Loading

**Problema**: Thumbnails carregados um por vez, não em paralelo.

**Solução**: Usar `rayon` para batch processing
```rust
use rayon::prelude::*;

fn load_thumbnails_batch(&self, paths: Vec<PathBuf>) {
    let sender = self.image_sender.clone();
    
    std::thread::spawn(move || {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED); }
        
        paths.par_iter()
            .take(50)  // MAX_CONCURRENT_LOADS
            .for_each(|path| {
                match extract_windows_thumbnail(path) {
                    Ok((data, w, h)) => {
                        let _ = sender.send(ThumbnailData {
                            path: path.clone(),
                            image_data: data,
                            width: w,
                            height: h,
                        });
                    }
                    Err(_) => {
                        // Send error placeholder
                    }
                }
            });
        
        unsafe { CoUninitialize(); }
    });
}
```

**Ganho Esperado**: 3-5x mais rápido em HDs, 10x+ em SSDs  
**Tempo Estimado**: 8 horas  
**Prioridade**: ⚠️ Média

---

### 🟢 Desejável

#### 7. Suporte a Thumbnail Caching em Disco

**Problema**: Recarrega thumbnails a cada execução.

**Solução**: Usar Windows Thumbnail Cache ou criar próprio
```rust
// Opção 1: Usar cache do Windows (já implementado!)
// IShellItemImageFactory já usa o cache do Explorer

// Opção 2: Cache local (se precisar de mais controle)
use std::collections::HashMap;
use std::fs;

struct DiskCache {
    cache_dir: PathBuf,
}

impl DiskCache {
    fn get(&self, path: &Path) -> Option<Vec<u8>> {
        let hash = hash_path(path);
        let cache_file = self.cache_dir.join(format!("{}.thumb", hash));
        fs::read(cache_file).ok()
    }
    
    fn put(&self, path: &Path, data: &[u8]) -> Result<()> {
        let hash = hash_path(path);
        let cache_file = self.cache_dir.join(format!("{}.thumb", hash));
        fs::write(cache_file, data)?;
        Ok(())
    }
}
```

**Tempo Estimado**: 10 horas  
**Prioridade**: 🟢 Baixa (já usa cache do Windows)

---

#### 8. Drag & Drop de Arquivos

**Problema**: Não é possível arrastar arquivos para outros apps.

**Solução**: Implementar `IDataObject` (Windows)
```rust
// egui tem suporte limitado a drag & drop
// Requer integração mais profunda com winit

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Detecta drag start
        if ui.drag_started() {
            frame.drag_window();  // Ou drag file
        }
    }
}
```

**Tempo Estimado**: 20 horas (complexo)  
**Prioridade**: 🟢 Baixa

---

#### 9. Preview de Vídeo (Play on Hover)

**Problema**: Vídeos mostram só thumbnail estático.

**Solução**: Reproduzir vídeo ao passar mouse
```rust
// Opção 1: Usar Windows Media Foundation
// Opção 2: Usar ffmpeg (adiciona 50 MB ao executável!)

// Por enquanto: Abrir vídeo em fullscreen ao clicar
```

**Tempo Estimado**: 40 horas (muito complexo)  
**Prioridade**: 🟢 Baixa

---

#### 10. Busca/Filtro de Arquivos

**Problema**: Impossível buscar em pastas grandes.

**Solução**: Barra de busca
```rust
struct ImageViewerApp {
    search_query: String,
    filtered_items: Vec<FileSystemItem>,
}

impl ImageViewerApp {
    fn filter_items(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_items = self.items.clone();
        } else {
            self.filtered_items = self.items
                .iter()
                .filter(|item| {
                    item.path()
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&self.search_query.to_lowercase())
                })
                .cloned()
                .collect();
        }
    }
}
```

**Tempo Estimado**: 4 horas  
**Prioridade**: 🟢 Média-Baixa

---

## Timeline Sugerido

### Sprint 1 (Semana 1-2) - Fundação

- [x] ✅ Criar documentação (`docs/`)
- [ ] 🔧 Refatorar em módulos (Critical)
- [ ] 🔧 Adicionar logging com `tracing`
- [ ] 🔧 Implementar testes unitários básicos

**Entregável**: Codebase profissional e testável

---

### Sprint 2 (Semana 3-4) - Segurança

- [ ] 🔒 Sanitização de paths
- [ ] 🔒 Validação de extensões no `ShellExecuteW`
- [ ] 🔒 Tratamento robusto de erros
- [ ] 🔒 Bloqueio de symlinks

**Entregável**: Aplicação segura para produção

---

### Sprint 3 (Semana 5-6) - Performance

- [ ] ⚡ Batch thumbnail loading com `rayon`
- [ ] ⚡ Otimização do LRU Cache
- [ ] ⚡ Profiling e benchmarks

**Entregável**: 3-5x melhoria em performance

---

### Sprint 4 (Semana 7-8) - UX

- [ ] 💾 Persistência de configurações
- [ ] 🔍 Busca/filtro de arquivos
- [ ] 🎨 Tema escuro
- [ ] ⌨️ Atalhos de teclado

**Entregável**: UX comparável ao Windows Explorer

---

## CI/CD Pipeline (Futuro)

```yaml
# .github/workflows/rust.yml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v2
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo test --all
      - run: cargo clippy -- -D warnings
      - run: cargo fmt -- --check
  
  build:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v2
      - run: cargo build --release
      - uses: actions/upload-artifact@v2
        with:
          name: mtt-file-manager.exe
          path: target/release/mtt-file-manager.exe
```

---

## Métricas de Sucesso

| Métrica | Atual | Meta (3 meses) |
|---------|-------|---------------|
| **Test Coverage** | 0% | 70% |
| **Build Time (Release)** | ~5 min | <2 min |
| **Startup Time** | ~500ms | <300ms |
| **Memory Usage (100 files)** | ~80 MB | <60 MB |
| **Thumbnails/sec** | ~20 | ~100 |
| **Lines of Code** | 675 | 1500 (bem estruturado) |
| **Cyclomatic Complexity** | Alta | <10 por função |

---

## Como Contribuir

1. Escolha um item do roadmap
2. Crie uma branch: `git checkout -b feature/nome-feature`
3. Implemente seguindo as regras em `.cursorrules`
4. Atualize a documentação em `docs/`
5. Adicione testes
6. Abra Pull Request

---

## Referências Técnicas

- [egui Best Practices](https://github.com/emilk/egui/blob/master/CONTRIBUTING.md)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Windows App Development Best Practices](https://docs.microsoft.com/en-us/windows/apps/design/)
