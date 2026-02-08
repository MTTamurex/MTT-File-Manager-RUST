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
│  │  │SQLite      │File System │Memory      │Directory   │Config    │  │  │
│  │  │Cache       │Access      │Cache       │Index       │Storage   │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │                   Worker Threads                                      │  │
│  │  ┌────────────┬────────────┬────────────┬────────────┬──────────┐  │  │
│  │  │Thumbnail   │File Ops    │Prefetch    │Folder      │Icon      │  │  │
│  │  │Workers     │Worker      │Worker      │Scanner   │Worker    │  │  │
│  │  └────────────┴────────────┴────────────┴────────────┴──────────┘  │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Camadas e Responsabilidades

### 1. Presentation Layer (UI Layer)
**Localização**: `src/ui/`

Responsável pela interface com o usuário usando eframe/egui (immediate mode GUI).

**Componentes principais**:
- **Toolbar**: Barra de ferramentas superior com botões de ação (`src/ui/toolbar.rs`)
- **Tab Bar**: Sistema de abas para navegação múltipla (`src/ui/tab_bar/mod.rs`)
- **File List/Grid**: Visualização de arquivos em grade ou lista (`src/ui/views/`)
- **Sidebar**: Painel lateral com atalhos e drives (`src/ui/sidebar.rs`)
- **Preview Panel**: Painel de preview de arquivos (`src/ui/preview_panel/`)
- **Status Bar**: Barra de status inferior (`src/ui/status_bar.rs`)

**Sub-módulos**:
- `src/ui/app/` - Ciclo de vida, input e notificações da aplicação
- `src/ui/components/` - Componentes reutilizáveis (media_preview, gif_manager, etc.)
- `src/ui/components/item_slot/` - Renderização de slots separada por tipo (drive/folder/file)
- `src/ui/components/mpv_preview/` - Bridge MPV separado por ciclo de vida/estado/embed
- `src/ui/tab_bar/` - Sistema de abas separado por renderer/controles/drag dwell
- `src/ui/views/` - Views principais (grid, list, computer)
- `src/ui/preview_panel/` - Sub-sistema de preview com suporte a vídeo

**Arquivos principais**:
- `src/ui/app_impl.rs` - Implementação principal do eframe::App
- `src/ui/app/input.rs` - Handler de input do usuário
- `src/ui/app/lifecycle.rs` - Ciclo de vida da aplicação
- `src/ui/tab_bar/mod.rs` - Sistema de abas (módulo coordenador)
- `src/ui/views/grid_view/mod.rs` - Visualização em grade
- `src/ui/views/list_view/` - Visualização em lista (com submódulos)
- `src/ui/views/computer_view.rs` - View "Este Computador"

### 2. Application Layer
**Localização**: `src/application/`

Contém a lógica de negócios e serviços da aplicação.

**Serviços principais**:
- **Navigation**: Gerenciamento de histórico de navegação (`src/application/navigation.rs`)
- **File Operations**: Operações de arquivo (copiar, mover, deletar) (`src/application/file_operations.rs`)
- **Clipboard Manager**: Gerenciamento da área de transferência (`src/application/clipboard.rs`)
- **Sorting Engine**: Motor de ordenação de arquivos (`src/application/sorting.rs`, `sorting_optimized.rs`)
- **Watcher Service**: Monitoramento de mudanças no filesystem (`src/application/watcher.rs`)
- **Notification System**: Sistema de notificações/toasts (`src/application/notification.rs`)
- **Renaming Service**: Lógica de renomeação (`src/application/renaming.rs`)
- **Context Menu**: Lógica do menu de contexto (`src/application/context_menu.rs`)

**Arquivos principais**:
- `src/application/navigation.rs` - Histórico de navegação
- `src/application/file_operations.rs` - Operações de arquivo
- `src/application/clipboard.rs` - Gerenciamento de clipboard
- `src/application/sorting_optimized.rs` - Ordenação otimizada (funções `sort_items`, `filter_items`)
- `src/application/notification.rs` - Sistema de notificações

### 3. Domain Layer
**Localização**: `src/domain/`

Define os modelos de dados e regras de negócio centrais.

**Modelos principais**:
- **FileEntry**: Representação de um arquivo/diretório (`src/domain/file_entry.rs`)
- **ThumbnailData**: Dados de thumbnail (`src/domain/thumbnail.rs`)
- **Error Types**: Tipos de erro da aplicação (`src/domain/errors.rs`)

**Enums importantes**:
- `SortMode { Name, Date, Size, Type, DriveTotalSpace, DriveFreeSpace }`
- `ViewMode { Grid, List }`
- `FoldersPosition { First, Last, Mixed }`
- `SyncStatus { None, CloudOnly, Syncing, Pinned, LocallyAvailable }`
- `IconSize { Small, Large, Jumbo }`

**Arquivos principais**:
- `src/domain/file_entry.rs` - Modelo FileEntry com DriveInfo
- `src/domain/thumbnail.rs` - Modelo de thumbnail
- `src/domain/errors.rs` - AppError enum e helpers

### 4. Infrastructure Layer
**Localização**: `src/infrastructure/`

Fornece acesso a recursos externos e serviços de sistema.

**Cache e Storage**:
- **`adaptive_batch.rs`** - Batch adaptativo para operações
- **`cache.rs`** - Cache genérico em memória
- **`cache_first.rs`** - Estratégia cache-first
- **`directory_cache.rs`** - Cache de diretórios
- **`directory_index.rs`** - Índice de diretórios para busca rápida
- **`disk_cache.rs`** - Cache em disco (SQLite) para thumbnails
- **`filesystem_cache.rs`** - Cache de filesystem
- **`io_priority.rs`** - Controle de prioridade de I/O
- **`ntfs_reader.rs`** - Leitor otimizado para NTFS
- **`virtual_drive_config.rs`** - Configuração de drives virtuais
- **`watcher.rs`** - Watcher genérico de filesystem
- **`windows_clipboard.rs`** - Integração nativa com clipboard Windows
- **`onedrive/mod.rs`** - Detecção de status OneDrive (path_detection, attributes, timeout_ops, directory_enum)
- **`security.rs`** - Validações de segurança

**Integrações Windows** (`src/infrastructure/windows/`):
- **`bitmap_conversion.rs`** - Conversão de bitmaps Windows
- **`codec_registry.rs`** - Registro de codecs de mídia
- **`device_change.rs`** - Monitoramento de mudanças de dispositivo

**Drive Watcher** (`src/infrastructure/`):
- **`drive_watcher.rs`** - Drive-wide file system watcher (ReadDirectoryChangesW)
  - Monitora drive inteiro (ex: `C:\`) ao invés de pasta individual
  - Async I/O com OVERLAPPED para não bloquear
  - Filtro de eventos por prefixo de pasta
- **`drive_watcher_integration.rs`** - Manager para múltiplos drives
  - Um watcher por drive (C:\, D:\, etc.)
  - Fallback para notify-watcher em UNC paths
- **`drives.rs`** - Gerenciamento de drives
- **`file_flags.rs`** - Flags de arquivo Windows
- **`file_system.rs`** - Operações de sistema de arquivos
- **`file_type.rs`** - Detecção de tipos de arquivo
- **`formatting.rs`** - Formatação de strings/números
- **`hdd_directory_reader.rs`** - Leitor otimizado de diretórios
- **`icons.rs`** - Extração de ícones do Windows
- **`iso_mount.rs`** - Montagem de arquivos ISO
- **`media_foundation.rs`** - Integração com Media Foundation
- **`native_menu.rs`** - Menu de contexto nativo
- **`recycle_bin.rs`** - Operações da lixeira
- **`shell_folder.rs`** - Pastas especiais do Shell
- **`shell_operations.rs`** - Operações do Shell (copiar, mover, deletar)
- **`system_info.rs`** - Informações do sistema
- **`window_subclass.rs`** - Subclasse de janela para customização
- **`metadata/`** - Metadados de imagem, vídeo e áudio

**Media** (`src/infrastructure/media/`):
- **`ffmpeg_session.rs`** - Sessão FFmpeg para extração de frames
- **`hardware_acceleration.rs`** - Detecção de aceleração por hardware
- **`tests_hw.rs`** - Testes de hardware

**Arquivos principais**:
- `src/infrastructure/windows/shell_operations.rs` - Operações de arquivo via Shell API
- `src/infrastructure/disk_cache.rs` - Cache SQLite
- `src/infrastructure/windows/icons.rs` - Extração de ícones

### 5. Workers Layer
**Localização**: `src/workers/`

Threads de background para processamento assíncrono.

**Workers disponíveis**:
- **`thumbnail/`** - Sistema de thumbnails multi-estágio
  - `extraction/stage1_image_crate.rs` - Stage 1: image crate
  - `extraction/stage2_wic.rs` - Stage 2: Windows Imaging Component
  - `extraction/stage3_shell_api.rs` - Stage 3: Shell API
  - `extraction/stage4_force_extract.rs` - Stage 4: Extração forçada
  - `extraction/stage5_media_foundation.rs` - Stage 5: Media Foundation
- **`thumbnail_loader.rs`** - Loader de thumbnails
- **`folder_scanner.rs`** - Scanner de pastas em background
- **`folder_preview_worker.rs`** - Geração de previews de pastas
- **`file_operation_worker.rs`** - Operações de arquivo assíncronas
- **`prefetch_worker.rs`** - Pré-carregamento de dados
- **`predictive_prefetch.rs`** - Prefetch preditivo
- **`idle_warmup.rs`** - Warmup de cache em idle

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
ImageViewerApp::new() [app/init.rs]
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

#### 1. Startup (main.rs → app/init.rs)
```rust
// main.rs
fn main() {
    // 1. Carrega ícone do app
    // 2. Configura viewport (borderless)
    // 3. Inicializa codec registry
    // 4. Chama eframe::run_native()
}

// app/init.rs - ImageViewerApp::new()
fn new(cc: &eframe::CreationContext) {
    // 1. Cria canais de comunicação (múltiplos workers)
    // 2. Inicializa workers threads (thumbnails, arquivos, ícones)
    // 3. Carrega preferências do SQLite
    // 4. Configura cache e índices
    // 5. Inicializa watchers (se feature notify-watcher habilitada)
    // 6. Carrega estado inicial
    // 7. Configura fontes customizadas
}
```

#### 2. Main Loop (ui/app_impl.rs)
```rust
fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
    // 1. Processa mensagens dos workers (thumbnails, arquivos, ícones, metadados)
    // 2. Processa eventos de filesystem (watcher)
    // 3. Atualiza estado da UI
    // 4. Processa input do usuário (teclado, mouse)
    // 5. Renderiza componentes
    // 6. Atualiza cache e thumbnails
    // 7. Gerencia animações (GIFs, vídeos)
}
```

#### 3. Shutdown
- Workers são finalizados quando canais são dropados
- Cache é persistido automaticamente
- Recursos COM são liberados via RAII

## Estado Global e Gerenciamento

### Estado Principal (ImageViewerApp)
**Localização**: `src/app/state.rs`

```rust
pub struct ImageViewerApp {
    // Navegação
    pub current_path: String,
    pub loaded_path: String,
    pub navigation: NavigationHistory,
    pub path_input: String,
    
    // Arquivos e seleção
    pub items: Arc<Vec<FileEntry>>,
    pub all_items: Vec<FileEntry>, // Cache mestre para busca
    pub selected_item: Option<usize>,
    pub selected_file: Option<FileEntry>,
    pub multi_selection: FxHashSet<PathBuf>,
    
    // Thumbnails e cache
    pub thumbnail_queue: Arc<PriorityThumbnailQueue>,
    pub image_receiver: Receiver<ThumbnailData>,
    pub pending_thumbnails: VecDeque<ThumbnailData>,
    pub disk_cache: Arc<ThumbnailDiskCache>,
    pub cache_manager: crate::ui::cache::CacheManager,
    
    // Async loading
    pub file_entry_receiver: Receiver<(usize, Vec<FileEntry>)>,
    pub is_loading_folder: bool,
    
    // Workers
    pub cover_worker_sender: Sender<PathBuf>,
    pub folder_preview_sender: Sender<PathBuf>,
    pub icon_req_sender: Sender<PathBuf>,
    pub metadata_req_sender: Sender<(PathBuf, u64)>,
    
    // UI State
    pub view_mode: ViewMode,
    pub thumbnail_size: f32,
    pub show_preview_panel: bool,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    
    // ... mais campos
}
```

### Sub-estados organizados
- **Cache State** (`src/app/cache_state.rs`) - Estado do cache
- **Navigation State** (`src/app/navigation_state.rs`) - Estado de navegação
- **UI State** (`src/app/ui_state.rs`) - Estado da interface
- **Worker State** (`src/app/worker_state.rs`) - Estado dos workers

### Roteamento de Telas
- **Computer View**: `src/ui/views/computer_view.rs`
- **Grid View**: `src/ui/views/grid_view/mod.rs`
- **List View**: `src/ui/views/list_view/mod.rs`
- **Recycle Bin View**: Renderização especial em `computer_view.rs`

### Comandos e Ações
- **Input Handler**: `src/ui/app/input.rs` e `src/app/operations/navigation/keyboard.rs`
- **Context Menu**: `src/ui/context_menu.rs` e `src/app/operations/context_menu.rs`
- **Keyboard Shortcuts**: Definidos em `input.rs` e `keyboard.rs`

## Comunicação entre Camadas

### Padrão MPSC (Multiple Producer, Single Consumer)
```rust
// UI → Worker (envia trabalho)
let (sender, receiver) = mpsc::channel();
worker_sender.send(work_item);

// Worker → UI (envia resultado)
ui_sender.send(result);

// UI recebe no update loop
while let Ok(result) = receiver.try_recv() {
    // Atualiza estado
}
```

### Workers e Canais
- **Thumbnail Worker**: `image_receiver` recebe `ThumbnailData`
- **File Entry Worker**: `file_entry_receiver` recebe `(generation, Vec<FileEntry>)`
- **Icon Worker**: `icon_res_receiver` recebe `(PathBuf, Vec<u8>, u32, u32)`
- **Metadata Worker**: `metadata_res_receiver` recebe `(PathBuf, u64, MediaMetadata)`
- **Cover Worker**: `cover_worker_receiver` recebe `(PathBuf, Option<PathBuf>)`
- **Folder Preview Worker**: `folder_preview_receiver` recebe `FolderPreviewData`

### Shared State
```rust
// Estado compartilhado com Arc
pub struct SharedState {
    pub cache: Arc<ThumbnailDiskCache>,
    pub directory_cache: Arc<DirectoryCache>,
    pub thumbnail_queue: Arc<PriorityThumbnailQueue>,
}
```

## Performance e Otimizações

### Workers Assíncronos
- **Thumbnail Workers**: Pool de threads com prioridade
- **File Operation Worker**: Thread dedicada para operações de arquivo
- **Prefetch Workers**: Pré-carregamento inteligente de pastas
- **Icon Worker**: Extração de ícones em background
- **Metadata Worker**: Extração de metadados em background

### Cache Multi-nível
1. **Texture Cache**: GPU textures no egui (mais rápido)
2. **Memory Cache**: LRU para acesso rápido (DashMap)
3. **Disk Cache**: SQLite para persistência (`disk_cache.rs`)
4. **Directory Cache**: Cache de estrutura de diretórios

### Virtualização
- **Grid Virtualization**: Renderização de itens visíveis apenas
- **List Virtualization**: Virtualização em list view
- **Scroll Prediction**: Predição de scroll para pré-carregamento
- **Adaptive Upload**: Throttling baseado em performance

### Thumbnails Multi-Estágio
1. Stage 1: image crate (PNG, JPG, GIF, WebP)
2. Stage 2: Windows Imaging Component (WIC)
3. Stage 3: Shell API (IShellItemImageFactory)
4. Stage 4: Extração forçada de frames
5. Stage 5: Media Foundation para vídeos

## Pontos de Extensão

### Novos Tipos de Preview
- Implementar em `src/ui/preview_panel/`
- Adicionar componente em `src/ui/components/`
- Registrar em `src/app/operations/view_setup.rs`

### Novas Operações de Arquivo
- Adicionar em `src/application/file_operations.rs`
- Implementar handler em `src/app/operations/file_ops.rs`
- Adicionar UI em toolbar/context menu

### Novas Integrações Windows
- Adicionar módulo em `src/infrastructure/windows/`
- Exportar em `src/infrastructure/windows/mod.rs`
- Seguir padrões de erro com `AppError`

### Novos Workers
- Criar em `src/workers/`
- Adicionar canais em `ImageViewerApp`
- Inicializar em `app/init.rs`
- Processar mensagens em `ui/app_impl.rs`

---

*Última atualização: 2026-02-08 (módulos modularizados em `mod.rs`)*

