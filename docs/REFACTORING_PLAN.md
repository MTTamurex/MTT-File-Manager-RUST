# 🔧 PLANO DE REFATORAÇÃO - main.rs

## MTT File Manager - Refatoração do Arquivo Monolítico

**Data de Criação**: Janeiro 2026  
**Status**: � Em Andamento  
**Versão**: 1.0

---

## ✅ Progresso das Fases

| Fase | Descrição | Status | Data |
|------|-----------|--------|------|
| **Fase 0** | Preparação | ✅ Completa | 13/01/2026 |
| **Fase 1** | Extração de Constantes e Theme | ✅ Completa | 13/01/2026 |
| **Fase 2** | Extração do State | ✅ Completa | 13/01/2026 |
| **Fase 3** | Extração de Operações | ✅ Completa | 13/01/2026 |
| **Fase 4** | Extração de UI Panels | ✅ Completa | 13/01/2026 |
| **Fase 5** | Extração do App Loop | ⬜ Pendente | - |
| **Fase 6** | Cleanup e Documentação | ⬜ Pendente | - |

---

## 📊 Diagnóstico Atual

### Problema Principal

O arquivo `main.rs` possui **~5000 linhas** de código, concentrando:

| Componente | Linhas Estimadas | Responsabilidade |
|------------|------------------|------------------|
| `struct ImageViewerApp` | ~200 | 50+ campos de estado |
| `fn new()` | ~400 | Inicialização de workers, caches, canais |
| Helpers de UI | ~300 | `icon_button()`, `toggle_icon_button()` |
| Operações de Arquivo | ~500 | Delete, Rename, Create folder |
| Clipboard | ~200 | Copy, Cut, Paste |
| Navegação | ~300 | Back, Forward, Up, Navigate |
| Ordenação/Filtro | ~150 | `sort_items()`, `filter_items()` |
| Renderização de Views | ~600 | Grid, List, Item slots |
| Preview Panel | ~300 | Painel de detalhes |
| Toolbar | ~300 | Barra de navegação |
| Context Menu handling | ~200 | Menu de contexto |
| Message Processing | ~400 | Processamento de mensagens de workers |
| Keyboard Shortcuts | ~150 | F5, Ctrl+C, Ctrl+V, etc. |
| `impl eframe::App` | ~500 | Loop principal de renderização |
| Watcher/Auto-refresh | ~100 | File system watcher |
| Resize/Window | ~200 | Bordas, grip, decorações |
| Notificações | ~100 | Toast system |
| **TOTAL** | **~5000** | - |

### Problemas Identificados

1. **God Object**: `ImageViewerApp` viola o Single Responsibility Principle
2. **Testabilidade Zero**: Impossível testar funções isoladamente
3. **Conflitos de Merge**: Qualquer mudança toca o mesmo arquivo
4. **Navegação Difícil**: Encontrar código específico leva tempo
5. **Duplicação Latente**: Código similar em diferentes funções
6. **Acoplamento Alto**: UI e lógica de negócios misturados

---

## 🎯 Objetivos da Refatoração

### Objetivos Primários

1. **Reduzir `main.rs` para <300 linhas** (apenas bootstrap e `fn main()`)
2. **Criar módulos coesos** com responsabilidade única
3. **Habilitar testes unitários** para lógica de negócios
4. **Melhorar manutenibilidade** de 3/5 para 4/5

### Objetivos Secundários

1. Eliminar código morto (`ui/app.rs`, `ui/core.rs` parciais)
2. Padronizar tratamento de erros
3. Centralizar constantes de UI (cores, tamanhos)
4. Documentar APIs públicas dos módulos

### Não-Objetivos (Fora do Escopo)

- Mudar bibliotecas (egui, windows-rs)
- Adicionar novas features
- Refatorar workers (já estão bons)
- Mudar arquitetura de cache

---

## 📁 Estrutura de Módulos Proposta

```
src/
├── main.rs                          # ~50 linhas (apenas bootstrap)
├── lib.rs                           # Re-exports
│
├── app/                             # 🆕 NOVA PASTA
│   ├── mod.rs                       # Re-exports
│   ├── state.rs                     # ImageViewerApp struct + campos
│   ├── init.rs                      # fn new() + inicialização de workers
│   ├── message_handler.rs           # process_incoming_messages()
│   └── shortcuts.rs                 # Keyboard shortcuts (F5, Ctrl+C, etc.)
│
├── application/                     # Casos de uso (já existe)
│   ├── mod.rs
│   ├── clipboard.rs                 # ✅ Já existe - expandir
│   ├── context_menu.rs              # ✅ Já existe - expandir
│   ├── navigation.rs                # ✅ Já existe - expandir
│   ├── file_operations.rs           # 🆕 Delete, Rename, Create
│   ├── notification.rs              # ✅ Já existe
│   ├── sorting.rs                   # 🆕 sort_items(), filter_items()
│   └── watcher.rs                   # ✅ Já existe - expandir
│
├── domain/                          # Entidades (já existe)
│   ├── mod.rs
│   ├── file_entry.rs                # ✅ Já existe
│   └── thumbnail.rs                 # ✅ Já existe
│
├── infrastructure/                  # APIs externas (já existe)
│   └── ...                          # ✅ Manter como está
│
├── workers/                         # Background threads (já existe)
│   └── ...                          # ✅ Manter como está
│
└── ui/                              # Interface (já existe)
    ├── mod.rs
    ├── app_impl.rs                  # 🆕 impl eframe::App
    ├── theme.rs                     # 🆕 Constantes de cores/tamanhos
    ├── widgets.rs                   # 🆕 icon_button(), toggle_icon_button()
    ├── toolbar.rs                   # 🆕 Barra de navegação
    ├── preview_panel.rs             # 🆕 Painel de detalhes
    ├── cache.rs                     # ✅ Já existe
    ├── sidebar.rs                   # ✅ Já existe
    ├── tab_bar.rs                   # ✅ Já existe
    ├── context_menu.rs              # ✅ Já existe
    ├── icon_loader.rs               # ✅ Já existe
    ├── svg_icons.rs                 # ✅ Já existe
    ├── components/                  # ✅ Já existe
    │   └── item_slot.rs
    └── views/                       # ✅ Já existe
        ├── grid_view.rs
        ├── list_view.rs
        └── computer_view.rs
```

---

## 📅 Fases de Implementação

### FASE 0: Preparação (1 dia)

**Objetivo**: Estabelecer baseline e infraestrutura de testes

| # | Tarefa | Risco | Estimativa |
|---|--------|-------|------------|
| 0.1 | Criar branch `refactor/main-rs-split` | Baixo | 5 min |
| 0.2 | Adicionar `cargo test` ao workflow | Baixo | 30 min |
| 0.3 | Documentar estado atual (linhas por seção) | Baixo | 1h |
| 0.4 | Criar snapshot de funcionamento (vídeo/GIF) | Baixo | 15 min |
| 0.5 | Backup do `main.rs` atual | Baixo | 5 min |

**Critério de Sucesso**: Branch criada, `cargo build --release` funciona

---

### FASE 1: Extração de Constantes e Theme (1 dia)

**Objetivo**: Centralizar magic numbers e cores

#### 1.1 Criar `ui/theme.rs`

```rust
// ui/theme.rs
use eframe::egui::Color32;

// === SPACING ===
pub const PADDING_XS: f32 = 2.0;
pub const PADDING_SM: f32 = 4.0;
pub const PADDING_MD: f32 = 8.0;
pub const PADDING_LG: f32 = 12.0;
pub const PADDING_XL: f32 = 16.0;

// === SIZES ===
pub const ICON_SIZE_SM: f32 = 16.0;
pub const ICON_SIZE_MD: f32 = 22.0;
pub const ICON_SIZE_LG: f32 = 24.0;
pub const THUMBNAIL_MIN: f32 = 64.0;
pub const THUMBNAIL_MAX: f32 = 256.0;
pub const THUMBNAIL_DEFAULT: f32 = 128.0;

// === COLORS (Light Mode) ===
pub const COLOR_SELECTION: Color32 = Color32::from_rgb(200, 220, 240);
pub const COLOR_SELECTION_TEXT: Color32 = Color32::from_rgb(0, 50, 100);
pub const COLOR_HOVER: Color32 = Color32::from_rgba_unmultiplied(200, 220, 240, 50);
pub const COLOR_ACCENT: Color32 = Color32::from_rgb(0, 120, 215);

// === COLORS (Dark Mode) ===
pub const COLOR_DARK_BG: Color32 = Color32::from_rgb(45, 45, 45);
pub const COLOR_DARK_HOVER: Color32 = Color32::from_white_alpha(30);

// === TIMING ===
pub const DEBOUNCE_MS: u64 = 50;
pub const DRIVE_REFRESH_MS: u64 = 350;
pub const AUTO_RELOAD_MS: u64 = 500;

// === CACHE SIZES ===
pub const TEXTURE_CACHE_SIZE: usize = 200;
pub const ICON_CACHE_SIZE: usize = 100;
pub const METADATA_CACHE_SIZE: usize = 512;
```

#### 1.2 Substituir magic numbers em `main.rs`

| Buscar | Substituir por |
|--------|----------------|
| `8.0` (padding) | `theme::PADDING_MD` |
| `22.0` (icon) | `theme::ICON_SIZE_MD` |
| `Color32::from_rgb(200, 220, 240)` | `theme::COLOR_SELECTION` |
| `Color32::from_rgb(45, 45, 45)` | `theme::COLOR_DARK_BG` |

**Riscos**:
- ⚠️ Alguns `8.0` podem não ser padding (verificar contexto)
- ✅ Mitigação: Usar busca manual, não find-replace global

**Critério de Sucesso**: Zero magic numbers restantes, visual idêntico

---

### FASE 2: Extração de Widgets Auxiliares (1 dia)

**Objetivo**: Mover helpers de UI para módulo dedicado

#### 2.1 Criar `ui/widgets.rs`

Extrair de `main.rs`:
- `fn icon_button()` (~50 linhas)
- `fn toggle_icon_button()` (~60 linhas)

```rust
// ui/widgets.rs
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use eframe::egui;

/// Renders an icon button with SVG support
pub fn icon_button(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    icon_name: &str,
    size: f32,
    tooltip: &str,
) -> egui::Response {
    // ... código extraído de main.rs
}

/// Renders a toggle button that shows active/inactive state
pub fn toggle_icon_button(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    icon_name: &str,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    // ... código extraído de main.rs
}
```

#### 2.2 Atualizar chamadas em `main.rs`

```rust
// Antes
self.icon_button(ui, ICON_ARROW_LEFT, "Voltar")

// Depois
use crate::ui::widgets;
widgets::icon_button(ui, &mut self.svg_icon_manager, "nav_back", 24.0, "Voltar")
```

**Riscos**:
- ⚠️ `icon_button` acessa `self.cache_manager` e `self.svg_icon_manager`
- ✅ Mitigação: Passar referências como parâmetros

**Critério de Sucesso**: Botões funcionam identicamente, código removido de `main.rs`

---

### FASE 3: Extração de Lógica de Negócios (2 dias)

**Objetivo**: Mover lógica não-UI para camada `application/`

#### 3.1 Expandir `application/clipboard.rs`

Mover de `main.rs`:
- `fn command_copy()` (~40 linhas)
- `fn command_cut()` (~30 linhas)
- `fn command_paste()` (~80 linhas)

```rust
// application/clipboard.rs
use std::path::PathBuf;
use crate::infrastructure::windows_clipboard;

pub struct ClipboardManager {
    internal_file: Option<PathBuf>,
    internal_op: Option<ClipboardOp>,
}

impl ClipboardManager {
    pub fn copy(&mut self, paths: &[PathBuf]) -> Result<(), ClipboardError> { ... }
    pub fn cut(&mut self, paths: &[PathBuf]) -> Result<(), ClipboardError> { ... }
    pub fn paste(&mut self, dest: &Path) -> Result<Vec<PathBuf>, ClipboardError> { ... }
}
```

#### 3.2 Criar `application/file_operations.rs`

Mover de `main.rs`:
- `fn delete_with_shell_for_idx()` (~40 linhas)
- `fn restore_from_recycle_bin()` (~40 linhas)
- `fn delete_permanently()` (~30 linhas)
- `fn empty_recycle_bin()` (~20 linhas)
- `fn create_new_folder()` (~50 linhas)
- `fn rename_with_shell()` (~50 linhas)
- `fn show_properties_for_idx()` (~15 linhas)

```rust
// application/file_operations.rs
use std::path::Path;
use crate::infrastructure::windows::shell_operations;

pub struct FileOperations {
    native_hwnd: Option<HWND>,
}

impl FileOperations {
    pub fn delete(&self, path: &Path, allow_undo: bool) -> Result<(), FileOpError> { ... }
    pub fn rename(&self, old: &Path, new_name: &str) -> Result<PathBuf, FileOpError> { ... }
    pub fn create_folder(&self, parent: &Path, name: &str) -> Result<PathBuf, FileOpError> { ... }
    pub fn show_properties(&self, path: &Path) -> Result<(), FileOpError> { ... }
}
```

#### 3.3 Criar `application/sorting.rs`

Mover de `main.rs`:
- `fn sort_items()` (~80 linhas)
- `fn filter_items()` (~20 linhas)

```rust
// application/sorting.rs
use crate::domain::file_entry::{FileEntry, SortMode, FoldersPosition};
use rayon::prelude::*;

/// Sorts items based on mode, direction, and folder position preference
pub fn sort_items(
    items: &mut [FileEntry],
    mode: SortMode,
    descending: bool,
    folders_position: FoldersPosition,
) {
    // Usa par_sort_by para >5000 itens
}

/// Filters items by search query (case-insensitive)
pub fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry> {
    // ...
}
```

#### 3.4 Expandir `application/navigation.rs`

Mover de `main.rs`:
- `fn navigate_to()` (~50 linhas)
- `fn go_back()` (~40 linhas)
- `fn go_forward()` (~40 linhas)
- `fn go_up_one_level()` (~20 linhas)
- `fn navigate_to_computer()` (~30 linhas)
- `fn navigate_to_recycle_bin()` (~30 linhas)
- `fn can_go_back()`, `fn can_go_forward()` (~10 linhas)

```rust
// application/navigation.rs
pub struct NavigationManager {
    history: Vec<String>,
    index: usize,
}

impl NavigationManager {
    pub fn navigate_to(&mut self, path: &str) { ... }
    pub fn go_back(&mut self) -> Option<&str> { ... }
    pub fn go_forward(&mut self) -> Option<&str> { ... }
    pub fn can_go_back(&self) -> bool { ... }
    pub fn can_go_forward(&self) -> bool { ... }
}
```

**Riscos**:
- ⚠️ Funções acessam muitos campos de `self` (side effects)
- ⚠️ `navigate_to()` chama `load_folder()`, `watch_current_folder()`, etc.
- ✅ Mitigação: Retornar `Action` enum ao invés de executar diretamente

```rust
pub enum NavigationAction {
    LoadFolder { path: String, force_refresh: bool },
    SetupComputerView,
    SetupRecycleBinView,
    UpdateWatcher { path: String },
}
```

**Critério de Sucesso**: Lógica testável isoladamente, `main.rs` apenas orquestra

---

### FASE 4: Extração de Componentes UI (2 dias)

**Objetivo**: Modularizar renderização de painéis

#### 4.1 Criar `ui/toolbar.rs`

Mover de `main.rs`:
- Renderização do `TopBottomPanel::top("nav_bar")` (~200 linhas)
- Barra de navegação, busca, controles de zoom/view

```rust
// ui/toolbar.rs
pub struct ToolbarState {
    pub search_query: String,
    pub is_address_editing: bool,
    pub path_input: String,
}

pub enum ToolbarAction {
    Navigate(String),
    GoBack,
    GoForward,
    GoUp,
    Refresh,
    CreateFolder,
    NavigateToComputer,
    ToggleViewMode,
    TogglePreviewPanel,
    ChangeSortMode(SortMode),
    ChangeZoom(f32),
    Search(String),
}

pub fn render_toolbar(
    ui: &mut egui::Ui,
    state: &mut ToolbarState,
    app_state: &AppState,
    svg_manager: &mut SvgIconManager,
) -> Option<ToolbarAction> {
    // ...
}
```

#### 4.2 Criar `ui/preview_panel.rs`

Mover de `main.rs`:
- Renderização do `SidePanel::right("preview_panel")` (~300 linhas)
- Preview de mídia, metadados, informações de arquivo

```rust
// ui/preview_panel.rs
pub fn render_preview_panel(
    ui: &mut egui::Ui,
    selected_file: Option<&FileEntry>,
    thumbnail: Option<&egui::TextureHandle>,
    metadata: Option<&MediaMetadata>,
    folder_size: Option<u64>,
    is_recycle_bin: bool,
) {
    // ...
}
```

#### 4.3 Criar `ui/app_impl.rs`

Mover de `main.rs`:
- `impl eframe::App for ImageViewerApp` (~500 linhas)
- `fn update()` - loop principal
- `fn on_exit()` - salvamento de preferências

```rust
// ui/app_impl.rs
use crate::app::ImageViewerApp;

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // 1. Process messages
        self.process_messages(ctx);
        
        // 2. Handle keyboard shortcuts
        self.handle_shortcuts(ctx);
        
        // 3. Render panels
        self.render_tab_bar(ctx);
        self.render_toolbar(ctx);
        self.render_sidebar(ctx);
        self.render_preview_panel(ctx);
        self.render_content(ctx);
        self.render_context_menu(ctx);
        self.render_notifications(ctx);
    }
    
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_preferences();
    }
}
```

**Riscos**:
- ⚠️ `update()` tem muita lógica condicional (if computer_view, if recycle_bin, etc.)
- ✅ Mitigação: Usar State pattern ou extrair condicionais para funções

**Critério de Sucesso**: Cada painel em seu próprio módulo, `app_impl.rs` apenas orquestra

---

### FASE 5: Extração do Estado da Aplicação (1 dia)

**Objetivo**: Criar `app/` com estado e inicialização separados

#### 5.1 Criar `app/state.rs`

Reorganizar `struct ImageViewerApp` em sub-structs:

```rust
// app/state.rs
use crate::application::{ClipboardManager, NavigationManager};
use crate::ui::cache::CacheManager;

/// Estado de UI (view mode, seleção, zoom)
pub struct UiState {
    pub view_mode: ViewMode,
    pub thumbnail_size: f32,
    pub show_preview_panel: bool,
    pub selected_item: Option<usize>,
    pub selected_file: Option<FileEntry>,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub scroll_to_selected: bool,
}

/// Estado de navegação
pub struct NavigationState {
    pub current_path: String,
    pub path_input: String,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub is_address_editing: bool,
    pub history: NavigationManager,
}

/// Estado de busca e ordenação
pub struct SearchSortState {
    pub search_query: String,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub folders_position: FoldersPosition,
}

/// Estado dos itens (arquivos/pastas)
pub struct ItemsState {
    pub items: Arc<Vec<FileEntry>>,
    pub all_items: Vec<FileEntry>,
    pub total_items: usize,
    pub is_loading_folder: bool,
    pub generation: usize,
}

/// Aplicação principal (composta de sub-estados)
pub struct ImageViewerApp {
    // Sub-estados
    pub ui: UiState,
    pub nav: NavigationState,
    pub search: SearchSortState,
    pub items: ItemsState,
    
    // Managers
    pub cache: CacheManager,
    pub clipboard: ClipboardManager,
    pub notifications: NotificationManager,
    
    // Workers (canais)
    pub workers: WorkerChannels,
    
    // Persistência
    pub disk_cache: Arc<ThumbnailDiskCache>,
    
    // Contexto egui
    pub ui_ctx: egui::Context,
    
    // ... outros campos específicos
}
```

#### 5.2 Criar `app/init.rs`

Mover `fn new()` (~400 linhas):

```rust
// app/init.rs
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        
        // 1. Initialize disk cache
        let disk_cache = init_disk_cache();
        
        // 2. Load preferences
        let prefs = load_preferences(&disk_cache);
        
        // 3. Spawn workers
        let workers = spawn_all_workers(ctx.clone(), disk_cache.clone());
        
        // 4. Initialize state
        Self {
            ui: UiState::new(&prefs),
            nav: NavigationState::new(&prefs),
            search: SearchSortState::new(&prefs),
            items: ItemsState::new(),
            cache: CacheManager::new(),
            clipboard: ClipboardManager::new(),
            notifications: NotificationManager::new(),
            workers,
            disk_cache,
            ui_ctx: ctx,
        }
    }
}

fn init_disk_cache() -> Arc<ThumbnailDiskCache> { ... }
fn load_preferences(cache: &ThumbnailDiskCache) -> AppPreferences { ... }
fn spawn_all_workers(...) -> WorkerChannels { ... }
```

#### 5.3 Criar `app/message_handler.rs`

Mover `fn process_incoming_messages()` (~400 linhas):

```rust
// app/message_handler.rs
impl ImageViewerApp {
    pub fn process_messages(&mut self, ctx: &egui::Context) {
        self.process_device_events();
        self.process_fs_events();
        self.process_file_entries();
        self.process_thumbnails();
        self.process_folder_previews();
        self.process_metadata();
        self.process_folder_sizes();
    }
    
    fn process_device_events(&mut self) { ... }
    fn process_fs_events(&mut self) { ... }
    // ...
}
```

**Riscos**:
- ⚠️ Refatoração grande, muitos arquivos novos
- ⚠️ Pode quebrar compilação temporariamente
- ✅ Mitigação: Fazer em pequenos commits, rodar `cargo check` frequentemente

**Critério de Sucesso**: `struct ImageViewerApp` legível, cada campo com propósito claro

---

### FASE 6: Limpeza e Finalização (1 dia)

**Objetivo**: Remover código morto, documentar, validar

#### 6.1 Remover código não utilizado

- Deletar `ui/app.rs` (versão incompleta)
- Deletar `ui/core.rs` (se não usado)
- Deletar `main.rs.backup`, `main.rs.refactored`, `main_functional_backup.rs`

#### 6.2 Documentar módulos públicos

```rust
//! # Application State Module
//! 
//! This module contains the main application state and its sub-components.
//! 
//! ## Architecture
//! 
//! ```text
//! ImageViewerApp
//! ├── UiState        - View mode, selection, zoom
//! ├── NavigationState - Current path, history
//! ├── SearchSortState - Search query, sort mode
//! ├── ItemsState     - File entries, loading state
//! ├── CacheManager   - Texture and icon caches
//! └── WorkerChannels - Communication with background threads
//! ```
```

#### 6.3 Validação final

| Teste | Comando | Esperado |
|-------|---------|----------|
| Compilação | `cargo build --release` | Sucesso, sem warnings |
| Lint | `cargo clippy -- -D warnings` | Sem erros |
| Formatação | `cargo fmt --check` | Sem diferenças |
| Funcional | Executar app manualmente | Todas features funcionando |

#### 6.4 Atualizar documentação

- Atualizar `AUDIT_REPORT.md` com notas de manutenibilidade
- Atualizar `README.md` com nova estrutura de módulos
- Criar/atualizar `docs/ARCHITECTURE.md` com diagrama atualizado

**Critério de Sucesso**: `main.rs` < 100 linhas, todos módulos documentados

---

## ⚠️ Matriz de Riscos

| Risco | Probabilidade | Impacto | Mitigação |
|-------|---------------|---------|-----------|
| Quebrar funcionalidade existente | Alta | Alto | Testar manualmente após cada fase |
| Conflitos de borrow checker | Média | Médio | Usar clones estratégicos, `Arc<>` |
| Performance regression | Baixa | Alto | Benchmark antes/depois |
| Tempo excedido | Média | Médio | Priorizar fases 1-3, adiar 4-5 se necessário |
| Código duplicado após split | Média | Baixo | Code review em cada PR |
| Dependências circulares | Média | Alto | Planejar imports antes de extrair |

---

## 📈 Métricas de Sucesso

### Antes vs Depois

| Métrica | Antes | Meta |
|---------|-------|------|
| Linhas em `main.rs` | ~5000 | <100 |
| Maior arquivo | 5000 (main.rs) | <500 |
| Módulos UI | 8 | 12+ |
| Módulos Application | 5 | 8+ |
| Testes unitários | 0 | 10+ |
| Cobertura de docs | ~10% | 50%+ |
| Nota Manutenibilidade | 3/5 | 4/5 |

### Checklist Final

- [ ] `main.rs` < 100 linhas
- [ ] Nenhum arquivo > 500 linhas
- [ ] `cargo clippy` passa sem warnings
- [ ] `cargo test` passa (quando testes existirem)
- [ ] Todas features funcionando identicamente
- [ ] README atualizado com nova estrutura
- [ ] AUDIT_REPORT atualizado

---

## 📅 Cronograma Estimado

| Fase | Duração | Dependências |
|------|---------|--------------|
| Fase 0: Preparação | 1 dia | - |
| Fase 1: Theme/Constantes | 1 dia | Fase 0 |
| Fase 2: Widgets | 1 dia | Fase 1 |
| Fase 3: Lógica de Negócios | 2 dias | Fase 2 |
| Fase 4: Componentes UI | 2 dias | Fase 3 |
| Fase 5: Estado da Aplicação | 1 dia | Fase 4 |
| Fase 6: Limpeza | 1 dia | Fase 5 |
| **TOTAL** | **9 dias** | - |

### Recomendação de Execução

1. **Sprint 1 (5 dias)**: Fases 0-3 (Preparação + Extração de lógica)
2. **Sprint 2 (4 dias)**: Fases 4-6 (UI + Finalização)

Ou, se tempo limitado:

1. **MVP (3 dias)**: Fases 0-2 (Theme + Widgets) - Ganho rápido
2. **Iteração futura**: Fases 3-6

---

## 📝 Notas Adicionais

### Padrões a Seguir

1. **Cada módulo < 300 linhas** (preferência)
2. **Funções < 50 linhas** (preferência)
3. **Documentar funções públicas** com `///`
4. **Usar `Result<T, E>`** ao invés de `panic!` ou `unwrap()`
5. **Preferir composição** sobre herança (traits)

### Código de Exemplo para Referência

Os módulos já existentes servem como referência de qualidade:
- `workers/thumbnail_worker.rs` - Bom exemplo de worker isolado
- `ui/sidebar.rs` - Bom exemplo de componente UI
- `infrastructure/disk_cache.rs` - Bom exemplo de persistência

### Contato para Dúvidas

Se houver dúvidas durante a implementação, revisar:
1. Este plano
2. `docs/AUDIT_REPORT.md`
3. Código existente nos módulos bem estruturados

---

*Plano criado em Janeiro 2026*  
*Última atualização: Janeiro 2026*
