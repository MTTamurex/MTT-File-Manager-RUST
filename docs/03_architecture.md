# Arquitetura - MTT File Manager

## Objetivo do Documento
Este documento descreve a arquitetura de alto nível do MTT File Manager, incluindo camadas, boundaries e ciclo de vida da aplicação.

## Visão Geral da Arquitetura

O MTT File Manager segue uma arquitetura em camadas com separação clara de responsabilidades:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Presentation Layer                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                           UI Layer                                     │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │  Toolbar   │  Tab Bar   │ File List  │   Sidebar  │ Preview  │  │  │
│  │  │  (Rust)    │  (Rust)    │  (Rust)    │  (Rust)    │ (Rust)   │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                    eframe/egui Framework                              │  │
│  │                    (Immediate Mode GUI)                              │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Application Layer                                  │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                    Application Services                                 │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │Navigation  │File Ops    │Clipboard   │Sorting     │Watcher   │  │  │
│  │  │History     │Manager     │Manager     │Engine      │Service   │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                      Domain Logic                                     │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │FileEntry   │Thumbnail   │SortMode    │ViewMode    │Errors    │  │  │
│  │  │Model       │Data        │Enum        │Enum        │Types     │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────────────────────┐
│                       Infrastructure Layer                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                    Windows Integration                                │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │Shell API   │File System │Media Found.│Thumbnail   │COM API   │  │  │
│  │  │Integration │Operations  │Integration │Extraction  │Wrapper   │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                     Data Layer                                        │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │SQLite      │File System │Memory      │Cache       │Config    │  │  │
│  │  │Cache       │Access      │Cache       │Manager     │Storage   │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                   Worker Threads                                      │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │Thumbnail   │File Ops    │Prefetch    │USN         │Preview   │  │  │
│  │  │Workers     │Worker      │Worker      │Watcher     │Worker    │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Camadas e Responsabilidades

### 1. Presentation Layer (UI Layer)
**Localização**: `src/ui/`

Responsável pela interface com o usuário usando eframe/egui (immediate mode GUI).

**Componentes principais**:
- **Toolbar**: Barra de ferramentas superior com botões de ação
- **Tab Bar**: Sistema de abas para navegação múltipla
- **File List/Grid**: Visualização de arquivos em grade ou lista
- **Sidebar**: Painel lateral com atalhos e drives
- **Preview Panel**: Painel de preview de arquivos
- **Status Bar**: Barra de status inferior

**Arquivos principais**:
- `src/ui/app_impl.rs` - Implementação principal do eframe::App
- `src/ui/toolbar.rs` - Componente toolbar
- `src/ui/tab_bar.rs` - Sistema de abas
- `src/ui/views/grid_view.rs` - Visualização em grade
- `src/ui/views/list_view.rs` - Visualização em lista
- `src/ui/sidebar.rs` - Sidebar
- `src/ui/preview_panel.rs` - Painel de preview

### 2. Application Layer
**Localização**: `src/application/`

Contém a lógica de negócios e serviços da aplicação.

**Serviços principais**:
- **Navigation**: Gerenciamento de histórico de navegação
- **File Operations**: Operações de arquivo (copiar, mover, deletar)
- **Clipboard Manager**: Gerenciamento da área de transferência
- **Sorting Engine**: Motor de ordenação de arquivos
- **Watcher Service**: Monitoramento de mudanças no filesystem

**Arquivos principais**:
- `src/application/navigation.rs` - Histórico de navegação
- `src/application/file_operations.rs` - Operações de arquivo
- `src/application/clipboard.rs` - Gerenciamento de clipboard
- `src/application/sorting.rs` - Ordenação
- `src/application/watcher.rs` - Monitoramento

### 3. Domain Layer
**Localização**: `src/domain/`

Define os modelos de dados e regras de negócio centrais.

**Modelos principais**:
- **FileEntry**: Representação de um arquivo/diretório
- **ThumbnailData**: Dados de thumbnail
- **Error Types**: Tipos de erro da aplicação

**Arquivos principais**:
- `src/domain/file_entry.rs` - Modelo FileEntry
- `src/domain/thumbnail.rs` - Modelo de thumbnail
- `src/domain/errors.rs` - Tipos de erro

### 4. Infrastructure Layer
**Localização**: `src/infrastructure/`

Fornece acesso a recursos externos e serviços de sistema.

**Integrações principais**:
- **Windows API**: Integração com Shell, COM, Media Foundation
- **File System**: Operações de I/O otimizadas
- **SQLite**: Cache persistente
- **Worker Threads**: Processamento assíncrono

**Arquivos principais**:
- `src/infrastructure/windows/` - Integrações Windows
- `src/infrastructure/disk_cache.rs` - Cache em disco
- `src/infrastructure/directory_cache.rs` - Cache de diretórios
- `src/workers/` - Workers assíncronos

## Principais Boundaries

### UI ↔ Application Boundary
- **Interface**: Traits e structs definidos em `src/app/`
- **Comunicação**: Channels MPSC para comunicação assíncrona
- **Estado**: Compartilhado via Arc<Mutex<>> e canais

### Application ↔ Infrastructure Boundary
- **Interface**: Funções públicas em módulos de infrastructure
- **Erros**: Conversão de erros via `thiserror` e `AppError`
- **Async**: Workers threads para operações de I/O

### Windows Integration Boundary
- **API**: windows-rs crate para bindings seguros
- **COM**: Inicialização e gerenciamento adequado de COM
- **Resources**: RAII para gerenciamento de recursos Windows

## Ciclo de Vida do App

```
main.rs
    ↓
ImageViewerApp::new() [init.rs]
    ↓
eframe::run_native()
    ↓
ImageViewerApp::update() [ui/app_impl.rs] ←──┐
    ↓                                      │
Process Input ──→ Update State ──→ Render UI │ (60 FPS loop)
    ↑                                      │
    └──────────────────────────────────────┘
```

### Fases Detalhadas

#### 1. Startup (main.rs → init.rs)
```rust
// main.rs
fn main() {
    // 1. Carrega ícone do app
    // 2. Configura viewport (borderless)
    // 3. Inicializa codec registry
    // 4. Chama eframe::run_native()
}

// init.rs - ImageViewerApp::new()
fn new(cc: &eframe::CreationContext) {
    // 1. Cria canais de comunicação
    // 2. Inicializa workers threads
    // 3. Carrega preferências do SQLite
    // 4. Configura cache e índices
    // 5. Inicializa watchers
    // 6. Carrega estado inicial
}
```

#### 2. Main Loop (ui/app_impl.rs)
```rust
fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
    // 1. Processa mensagens dos workers
    // 2. Atualiza estado da UI
    // 3. Processa input do usuário
    // 4. Renderiza componentes
    // 5. Atualiza cache e thumbnails
}
```

#### 3. Shutdown
- Workers são finalizados quando canais são dropados
- Cache é persistido automaticamente
- Recursos COM são liberados

## Estado Global e Gerenciamento

### Estado Principal (ImageViewerApp)
**Localização**: `src/app/state.rs`

```rust
pub struct ImageViewerApp {
    // Navegação
    current_path: String,
    navigation: NavigationHistory,
    
    // Arquivos e seleção
    items: Arc<Vec<FileEntry>>,
    selected_item: Option<usize>,
    multi_selection: FxHashSet<PathBuf>,
    
    // Cache e workers
    thumbnail_queue: Arc<PriorityThumbnailQueue>,
    disk_cache: Arc<ThumbnailDiskCache>,
    
    // UI State
    view_mode: ViewMode,
    thumbnail_size: f32,
    show_preview_panel: bool,
    
    // ... mais campos
}
```

### Roteamento de Telas
- **Computer View**: `src/ui/views/computer_view.rs`
- **Grid View**: `src/ui/views/grid_view.rs`
- **List View**: `src/ui/views/list_view.rs`

### Comandos e Ações
- **Input Handler**: `src/ui/app/input.rs`
- **Context Menu**: `src/ui/context_menu.rs`
- **Keyboard Shortcuts**: Definidos em `input.rs`

## Comunicação entre Camadas

### Padrão MPSC (Multiple Producer, Single Consumer)
```rust
// UI → Worker
let (tx, rx) = mpsc::channel();
// Envia trabalho
worker_sender.send(work_item);

// Worker → UI
// Worker processa e envia resultado
ui_sender.send(result);
// UI recebe no update loop
while let Ok(result) = receiver.try_recv() {
    // Atualiza estado
}
```

### Shared State
```rust
// Estado compartilhado com Arc<Mutex<>>
pub struct SharedState {
    pub cache: Arc<Mutex<CacheManager>>,
    pub config: Arc<Mutex<Config>>,
}
```

## Performance e Otimizações

### Workers Assíncronos
- **Thumbnail Workers**: 8 threads para geração de thumbnails
- **File Operation Worker**: Thread dedicada para operações de arquivo
- **Prefetch Workers**: Pré-carregamento inteligente

### Cache Multi-nível
1. **Memory Cache**: LRU para acesso rápido
2. **Disk Cache**: SQLite para persistência
3. **Texture Cache**: GPU textures no egui

### Virtualização
- **Grid Virtualization**: Renderização de itens visíveis apenas
- **Scroll Prediction**: Predição de scroll para pré-carregamento
- **Adaptive Upload**: Throttling baseado em performance

## Pontos de Extensão

### Novos Tipos de Preview
- Implementar trait em `src/ui/components/media_preview.rs`
- Adicionar worker em `src/workers/`
- Registrar em `src/app/operations/`

### Novas Operações de Arquivo
- Adicionar em `src/application/file_operations.rs`
- Implementar handler em `src/app/operations/file_ops.rs`

### Novas Integrações Windows
- Adicionar módulo em `src/infrastructure/windows/`
- Exportar em `src/infrastructure/windows/mod.rs`