# 🗺️ Roadmap Técnico - MTT File Manager

## ✅ Features Implementadas (Atualizado: 2026-01-01)

### Progresso da Refatoração Modular
- **Sprint 1: Infrastructure** ✅ **CONCLUÍDO**
  - Módulos Windows movidos para `src/infrastructure/windows/`
  - Imports corrigidos em todos os módulos
  - Código compila sem erros

- **Sprint 2: Workers** ✅ **CONCLUÍDO**
  - `thumbnail_worker.rs` criado e integrado
  - Redução de ~40 linhas em `main.rs`
  - Workers funcionando corretamente

- **Sprint 3: UI Components** 🚧 **EM ANDAMENTO**
  - `grid_view.rs` e `list_view.rs` extraídos e integrados
  - `sidebar.rs` extraído e integrado
  - `computer_view.rs` extraído (pronto para integração)
  - Redução de ~400 linhas em `main.rs`

### Novas Features Implementadas
- **Sistema de Notificações**: Toast no canto inferior direito com fade animation
- **Icon Loader Assíncrono**: Carregamento de ícones em worker thread separada
- **CacheManager Unificado**: Gerenciamento centralizado de caches
- **Smart Sorting com `natord`**: Ordenação natural ("File1, File2, File10")

### Ícones Nativos do Windows
- **SHGetFileInfoW** para ícones de arquivos por extensão (SHGFI_USEFILEATTRIBUTES)
- **SHGetFileInfoW** para ícones de pastas (dummy path + FILE_ATTRIBUTE_DIRECTORY)
- **SHGetFileInfoW** para ícones reais de drives (path real, sem USEFILEATTRIBUTES)
- **GetVolumeInformationW** para labels de volumes (ex: "Sistema (C:)")
- **Cache LRU**: icon_cache, folder_icon_texture, drive_icon_cache

### Menu de Contexto (Right-Click)
- **Detecção de clique direito** em itens do grid e lista
- **Popup nativo do egui** com posicionamento preciso
- **Operações integradas**: Copiar (Ctrl+C), Recortar (Ctrl+X), Colar (Ctrl+V), Renomear (F2), Excluir (Delete)
- **Estado de contexto**: Armazenamento de posição e item selecionado
- **Integração com funções existentes**: Reutiliza `command_copy`, `command_cut`, `command_paste`, `delete_with_shell`, `rename_with_shell`

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

### Otimização de VRAM ✅
- CACHE_SIZE reduzido de 500 → 100 (Otimizado para manter ~100MB VRAM)
- MAX_CONCURRENT_DECODES limitado a 4 (Proteção contra picos de RAM)
- Worker threads reduzidas para 4 (Performance balanceada)
- Resize antecipado (Transient Flow) para liberar RAM de alta resolução
### Estabilidade e Gerenciamento de Ciclo de Vida (Anti-Leak)
- **Persistent Worker Pool**: Fila de 4 threads para evitar disk thrashing em HDDs externos.
- **Atomic Generational Validation**: Sistema de gerações que invalida tasks de thumbnails via `Arc<AtomicUsize>`.
- **Reactive Repaints**: Workers agora disparam `ctx.request_repaint()` imediatamente ao completar tarefas, garantindo fluidez sem depender de inputs do usuário.
- **CreationContext Initialization**: Refatorada inicialização para captar o contexto egui nativamente.
- **Native Rename (F2)**: Implementado renomeação via `SHFileOperationW` com suporte a Undo (Ctrl+Z).
- **Manual & Auto Refresh**: Sistema de recarga via F5 e monitoramento automático via `notify` crate com debounce de 500ms.
- **Delete to Recycle Bin**: Implementado via `SHFileOperationW` (`FO_DELETE`) com suporte nativo do SO.
- **Create New Folder**: Fluxo instantâneo com auto-rename e resolução de conflitos de nome.
- **Clipboard Operations (Ctrl+C/X/V)**: Copy, Cut e Paste via `Event::Copy/Cut/Paste` do egui e `SHFileOperationW` (`FO_COPY`/`FO_MOVE`).
- **UI Refinements**: Ícones Remix Icon corrigidos, layout responsivo e botões frameless.
- **Persistent Sort & Folder Position ✅**: Preferências de ordenação e posição de pastas salvas em SQLite.
- **Proactive Folder Previews ✅**: Escaneamento de capas de pastas no modo lista e persistência em SQLite.
- **Native Folder Previews (“Sandwich Effect”) ✅**: Implementado via `IShellItemImageFactory` com sistema de carregamento assíncrono e cache dedicado.

### Otimização do Garbage Collector (2026-01-02) ✅ CONCLUÍDO
- **Problema Crítico Resolvido**: GC bloqueava database lock durante verificações de arquivos (I/O lento), travando navegação.
- **Solução Implementada**: Refatoração em 3 fases com locks curtos:
  1. **Fase 1**: Leitura rápida de paths (lock ~50ms)
  2. **Fase 2**: Verificação de existência de arquivos (SEM lock)
  3. **Fase 3**: Batch transaction para deleções (1 commit vs N commits)
- **Correção Adicional**: `setup_computer_view()` agora seta `is_loading_folder = false`
- **Impacto**: App responde imediatamente durante GC, deleções 10-100x mais rápidas

### Detecção Instantânea de USB Drives (2026-01-04) ✅ CONCLUÍDO
- **Problema Original**: USB drives não eram detectados automaticamente ao conectar/remover
- **Solução Implementada**: Sistema nativo de detecção via `WM_DEVICECHANGE`
  - Worker thread dedicado com janela `HWND_MESSAGE` oculta
  - Registro de `GUID_DEVINTERFACE_VOLUME` via `RegisterDeviceNotificationW`
  - Comunicação assíncrona via `mpsc::channel` + `egui::Context.request_repaint()`
  - Polling de fallback (350ms) para casos extremos
- **Desafio Resolvido**: egui em modo reactive aguardava input - solução foi chamar `request_repaint()` diretamente do worker thread
- **Resultado**: Detecção <100ms, atualização imediata da UI sem aguardar eventos de mouse/teclado
- **Localização**: `src/infrastructure/windows/device_change.rs`

---

## Débitos Técnicos Identificados (Atualizado: 2026-01-04)

### 🔴 Crítico

#### 1. Monolito de 3134 Linhas (Progresso Significativo)

**Status**: 🚧 **EM REFATORAÇÃO ATIVA**  
**Progresso**: ~15 módulos extraídos, ~440 linhas movidas de `main.rs`

**Impacto Atual**:
- ✅ Módulos Windows extraídos para `infrastructure/windows/`
- ✅ Workers extraídos para módulos dedicados
- ✅ Views (grid, list, sidebar) extraídas
- ⚠️ `main.rs` ainda grande (~3134 linhas)
- ⚠️ Alguns componentes ainda inline

**Próximos Passos**:
- Completar Sprint 3 (extrair context menu, integrar computer_view)
- Iniciar Sprint 4 (extrair top bar, status bar)
- Meta: `main.rs` < 2000 linhas

**Prioridade**: 🔥 Alta (em progresso)

---

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

#### 7. Suporte a Thumbnail Caching em SQLite ✅ **CONCLUÍDO**

**Problema**: Recarregava thumbnails a cada execução ou ocupava espaço excessivo com arquivos pequenos fragmentados.

**Solução**: Implementado `ThumbnailDiskCache` persistente via SQLite em `thumbnails.db`.
- **Performance**: WAL mode habilitado para I/O concorrente entre workers.
- **Espaço**: Redução de >80% via consolidação em banco de dados e WebP otimizado (200px max).
- **Inteligência**: Invalidação automática por path hash + timestamp de modificação.
- **Migração**: Sistema limpa automaticamente arquivos legados na inicialização.

**Ganho**: Carregamento instantâneo e uso de disco otimizado.


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

- [x] 💾 Persistência de configurações
- [x] 🔍 Busca/filtro de arquivos
- [x] 🎨 Tema escuro (suporte via visuals)
- [x] ⌨️ Atalhos de teclado (Delete, F2, F5, Ctrl+Shift+N)

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
