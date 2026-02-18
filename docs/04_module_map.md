# Mapa do Repositório - MTT File Manager

## Objetivo do Documento
Este documento fornece um mapa detalhado do repositório, listando diretórios, responsabilidades e principais componentes de cada módulo.

## Estrutura de Diretórios

O projeto é um Cargo Workspace com 3 crates:

```
MTT-File-Manager-RUST/
├── Cargo.toml                        # Workspace root + pacote mtt-file-manager
├── src/                              # App principal (mtt-file-manager)
│   ├── app/                          # Estado e lógica principal da aplicação
│   ├── application/                  # Serviços de lógica de negócios
│   ├── domain/                       # Modelos de dados e regras de negócio
│   ├── infrastructure/               # Integrações com sistema e recursos externos
│   ├── pdf_viewer/                   # Visualizador de PDF externo (WebView2)
│   ├── tabs/                         # Sistema de abas
│   ├── ui/                           # Interface do usuário
│   ├── workers/                      # Threads de background
│   ├── embedded_assets.rs            # Recursos embarcados (fontes)
│   ├── lib.rs                        # Entry point da lib
│   └── main.rs                       # Entry point do binário
├── crates/
│   ├── mtt-search-protocol/          # Tipos IPC compartilhados
│   └── mtt-search-service/           # Windows Service de indexação híbrida
```

## Módulos Detalhados

### 1. `src/app/` - Estado e Lógica Principal
**Propósito**: Gerenciamento de estado global, inicialização e operações da aplicação

**Arquivos de estado**:
- **`state.rs`** - Struct `ImageViewerApp` com estado completo da aplicação (principal)
- **`state_new.rs`** - Nova implementação de estado (em transição)
- **`init.rs`** - Inicialização e criação de workers/canais
- **`cache_state.rs`** - Gerenciamento de estado do cache
- **`navigation_state.rs`** - Estado de navegação
- **`ui_state.rs`** - Estado da interface
- **`worker_state.rs`** - Estado dos workers
- **`mod.rs`** - Re-exports do módulo

**Operações** (`src/app/operations/`):
- **`mod.rs`** - Re-exports e traits
- **`clipboard_ops.rs`** - Operações de clipboard (copiar, cortar, colar)
- **`context_menu.rs`** - Menu de contexto
- **`file_ops.rs`** - Operações de arquivo (deletar, renomear)
- **`folder_loading/mod.rs`** - Carregamento de pastas
- **`folder_lock_ops.rs`** - Operações de bloqueio de view por pasta
- **`pinned_folder_ops.rs`** - Fixar/desafixar/reordenar pastas no Acesso Rápido
- **`icons.rs`** - Gerenciamento de ícones
- **`message_handler/mod.rs`** - Handler de mensagens entre threads
- **`metadata.rs`** - Solicitação de metadados
- **`preferences.rs`** - Preferências do usuário
- **`recycle_bin_ops.rs`** - Operações da lixeira
- **`selection.rs`** - Seleção de arquivos
- **`tabs.rs`** - Gerenciamento de abas
- **`thumbnails.rs`** - Solicitação de thumbnails
- **`trait_impls.rs`** - Implementações de traits
- **`view_setup.rs`** - Configuração de views
- **`watcher.rs`** - Monitoramento de mudanças
- **`window.rs`** - Gerenciamento de janela

**Submódulos de operações refatorados**:
- **`folder_loading/`** - Pipeline de carregamento de pastas
  - **`mod.rs`** - Coordenador do fluxo
  - **`load_pipeline.rs`** - Pipeline principal de carregamento
  - **`load_pipeline/fast_paths.rs`** - Fast-paths e heuristicas de carregamento
  - **`load_pipeline/optimized_tiers.rs`** - Tiers otimizados de carregamento
  - **`load_pipeline/tier3_fallback.rs`** - Fallback robusto (OneDrive timeout/FindFirstFileW)
  - **`folder_scan.rs`** - Scan e leitura de diretórios
  - **`refresh.rs`** - Regras de refresh e reload
  - **`guards.rs`** - Guards/validações de fluxo
  - **`view_updates.rs`** - Atualizações de estado para UI
- **`message_handler/`** - Processamento de eventos dos workers/watchers
  - **`mod.rs`** - Dispatcher/orquestração
  - **`file_op_events.rs`** - Eventos de operações de arquivo
  - **`global_search_events.rs`** - Eventos da busca global
  - **`watcher_events.rs`** - Eventos de file watcher
  - **`watcher_drive_processing.rs`** - Processamento de eventos de drive watcher
  - **`watcher_reload.rs`** - Politica de reload/refresh final
  - **`watcher_legacy.rs`** - Fallback legado (`notify`) para cenarios especificos
  - **`thumbnail_events.rs`** - Eventos de thumbnail
  - **`thumbnail_workers.rs`** - Drains e integracao com workers de thumbnail
  - **`thumbnail_uploads.rs`** - Pipeline de upload de thumbnails/previews para UI
  - **`thumbnail_rebuild.rs`** - Rebuild incremental de itens/miniaturas
  - **`rebuild_events.rs`** - Eventos de rebuild/reordenação
  - **`helpers.rs`** - Helpers e utilitários de apoio

**Navegação** (`src/app/operations/navigation/`):
- **`mod.rs`** - Lógica principal de navegação
- **`keyboard.rs`** - Navegação por teclado
- **`selection.rs`** - Seleção via navegação

**UI Rendering** (`src/app/operations/ui_rendering/`):
- **`mod.rs`** - Coordenação de renderização
- **`grid_bridge.rs`** - Bridge para grid view
- **`item_slot_bridge.rs`** - Bridge para item slots
- **`list_bridge.rs`** - Bridge para list view

**Principais structs/enums**:
```rust
pub struct ImageViewerApp { /* estado completo */ }
pub enum LastInput { Mouse, Keyboard }
pub struct ItemsRebuildResult { generation, request_id, items, total_items }
```

**Dependências**: Todos os outros módulos

---

### 2. `src/application/` - Serviços de Negócio
**Propósito**: Lógica de negócios pura, sem dependência de UI

**Arquivos principais**:
- **`mod.rs`** - Re-exports do módulo
- **`clipboard.rs`** - Gerenciamento de clipboard (ClipboardManager)
- **`context_menu.rs`** - Lógica do menu de contexto
- **`file_operations.rs`** - Operações de arquivo de alto nível
- **`navigation.rs`** - Histórico de navegação (NavigationHistory)
- **`notification.rs`** - Sistema de notificações/toasts (NotificationManager)
- **`renaming.rs`** - Lógica de renomeação
- **`sorting.rs`** - Fachada de ordenação/filtro (API pública)
- **`sorting/sort_impl.rs`** - Implementação de ordenação
- **`sorting/filtering.rs`** - Implementação de filtros
- **`state.rs`** - Estado da aplicação (ApplicationState)
- **`watcher.rs`** - Watcher de filesystem

**Principais structs**:
```rust
pub struct NavigationHistory { /* histórico linear */ }
pub struct ClipboardManager { /* clipboard */ }
pub struct NotificationManager { /* notificações toast */ }
pub struct ApplicationState { /* estado persistente */ }
```

**Funções otimizadas exportadas**:
```rust
pub fn sort_items(items: &mut [FileEntry], mode: SortMode, descending: bool, folders_first: bool)
pub fn filter_items(items: &[FileEntry], query: &str) -> Vec<FileEntry>
pub fn filter_items_cow<'a>(items: &'a [FileEntry], query: &str) -> Cow<'a, [FileEntry]>
```

**Dependências**: `domain`, `infrastructure`

---

### 3. `src/domain/` - Modelos de Dados
**Propósito**: Modelos de dados e regras de negócio centrais (domínio puro)

**Arquivos principais**:
- **`mod.rs`** - Re-exports do módulo
- **`errors.rs`** - Tipos de erro da aplicação (AppError)
- **`file_entry.rs`** - Modelo FileEntry e enums relacionados
- **`folder_lock.rs`** - Preferências de view por pasta (FolderLock)
- **`pinned_folder.rs`** - Pasta fixada no Acesso Rápido (PinnedFolder)
- **`thumbnail.rs`** - Modelo de thumbnail (ThumbnailData)

**Principais structs/enums**:
```rust
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub folder_cover: Option<PathBuf>,
    pub drive_info: Option<DriveInfo>,
    pub sync_status: SyncStatus,
    pub deletion_date: Option<String>,
    pub recycle_original_path: Option<PathBuf>,
}

pub struct DriveInfo {
    pub file_system: String,
    pub total_space: u64,
    pub free_space: u64,
    pub drive_type: DriveType,
}

pub enum SortMode { Name, Date, Size, Type, DriveTotalSpace, DriveFreeSpace }
pub enum ViewMode { Grid, List }
pub enum FoldersPosition { First, Last, Mixed }
pub enum SyncStatus { None, CloudOnly, Syncing, Pinned, LocallyAvailable }
pub enum IconSize { Small, Large, Jumbo }

pub enum AppError {
    Security(SecurityError),
    WindowsApi(String),
    Io(std::io::Error),
    ThumbnailExtraction { path: PathBuf, source: anyhow::Error },
    FileOperation(String),
    InvalidState(String),
    Config(String),
    Worker(String),
    UiRendering(String),
}
```

**Dependências**: Apenas crates externos (std, etc)

---

### 4. `src/infrastructure/` - Integrações Externas
**Propósito**: Acesso a recursos externos e integrações de sistema

**Cache e Storage**:
- **`mod.rs`** - Re-exports
- **`adaptive_batch.rs`** - Batch adaptativo para operações
- **`cache.rs`** - Cache genérico em memória
- **`cache_first.rs`** - Estratégia cache-first
- **`directory_cache.rs`** - Cache de diretórios
- **`directory_index.rs`** - Índice de diretórios
- **`disk_cache.rs`** - Cache em disco (SQLite) - ThumbnailDiskCache
- **`filesystem_cache.rs`** - Cache de filesystem
- **`io_priority.rs`** - Prioridade de I/O
- **`ntfs_reader.rs`** - Leitor NTFS otimizado
- **`drive_watcher.rs`** - Drive-wide file watcher (ReadDirectoryChangesW)
- **`drive_watcher_integration.rs`** - Manager para múltiplos drive watchers
- **`onedrive/mod.rs`** - Detecção de status OneDrive
- **`security.rs`** - Validações de segurança
- **`virtual_drive_config.rs`** - Configuração de drives virtuais
- **`watcher.rs`** - Watcher genérico de filesystem
- **`windows_clipboard.rs`** - Clipboard Windows (CF_HDROP)
- **`global_search.rs`** - Cliente IPC (Named Pipe) para o serviço de busca global

**Submódulos refatorados**:
- **`onedrive/`** - Módulo OneDrive segmentado por responsabilidade
  - **`mod.rs`** - API pública/coordenador
  - **`path_detection.rs`** - Detecção de caminhos OneDrive
  - **`attributes.rs`** - Atributos/sync flags
  - **`timeout_ops.rs`** - Operações com timeout
  - **`directory_enum.rs`** - Enumeração de diretórios
- **`disk_cache/`** - Repositórios de persistência e GC segmentados
  - **`thumbnails_repo.rs`** - CRUD de thumbnails
  - **`folder_previews.rs`** - CRUD de previews de pasta
  - **`folder_covers.rs`** - CRUD de capas de pasta
  - **`folder_locks.rs`** - CRUD de preferências de view por pasta
  - **`pinned_folders.rs`** - CRUD de pastas fixadas no Acesso Rápido
  - **`preferences.rs`** - Preferências persistidas
  - **`cleanup.rs`** - Limpeza/manutenção de cache
  - **`gc.rs`** - Coleta incremental/full + vacuum

**Integrações Windows** (`src/infrastructure/windows/`):
- **`mod.rs`** - Re-exports e funções principais
- **`bitmap_conversion.rs`** - Conversão HBITMAP → RGBA
- **`codec_registry.rs`** - Registro de codecs de mídia
- **`device_change.rs`** - Monitoramento de dispositivos
- **`drives.rs`** - Enumeração de drives
- **`file_flags.rs`** - Flags de arquivo Windows
- **`file_system.rs`** - Operações de filesystem
- **`file_type.rs`** - Detecção de tipos MIME
- **`formatting.rs`** - Formatação de bytes, datas
- **`hdd_directory_reader.rs`** - Leitor otimizado de diretórios
- **`icons.rs`** - Extração de ícones (IImageList, IShellItemImageFactory)
- **`iso_mount.rs`** - Montagem de ISOs
- **`media_foundation.rs`** - Integração Media Foundation
- **`native_menu.rs`** - Menu de contexto nativo (IContextMenu)
- **`recycle_bin.rs`** - Operações da lixeira
- **`shell_folder.rs`** - Pastas especiais do Shell
- **`shell_operations.rs`** - Operações do Shell (IFileOperation)
- **`system_info.rs`** - Informações do sistema
- **`window_subclass.rs`** - Subclasse de janela

**Metadata** (`src/infrastructure/windows/metadata/`):
- **`mod.rs`** - Re-exports
- **`audio_sniffing.rs`** - Detecção de metadados de áudio
- **`image.rs`** - Metadados de imagem
- **`property_keys.rs`** - Constantes de propriedades Windows
- **`utils.rs`** - Utilitários de metadata
- **`video_sniffing.rs`** - Detecção de metadados de vídeo
- **`video.rs`** - Metadados de vídeo

**Media** (`src/infrastructure/media/`):
- **`mod.rs`** - Re-exports
- **`ffmpeg_session.rs`** - Sessão FFmpeg para extração
- **`hardware_acceleration.rs`** - Detecção de aceleração por hardware
- **`tests_hw.rs`** - Testes de hardware

**Principais funções**:
```rust
pub fn get_all_drives() -> Vec<(String, String)>
pub fn extract_file_icon_by_path(path: &Path, size: i32) -> Result<(Vec<u8>, u32, u32)>
pub fn extract_thumbnail_with_media_foundation(path: &Path) -> Result<DynamicImage>
pub fn copy_items_via_shell(src: &[PathBuf], dest: &Path) -> Result<()>
pub fn move_items_via_shell(src: &[PathBuf], dest: &Path) -> Result<()>
pub fn delete_items_via_shell(paths: &[PathBuf]) -> Result<()>

// Drive Watcher (File Pilot optimization)
pub struct DriveWatcherManager
pub fn watch_path(&mut self, path: PathBuf)
pub fn poll_events(&self) -> Vec<DriveWatcherEvent>
pub fn is_active(&self) -> bool
```

**Dependências**: Crates externos (windows, rusqlite, etc)

---

### 5. `src/ui/` - Interface do Usuário
**Propósito**: Componentes de interface e renderização

**App** (`src/ui/app/`):
- **`mod.rs`** - Re-exports
- **`input.rs`** - Handler de input (teclado, mouse)
- **`lifecycle.rs`** - Ciclo de vida da aplicação
- **`menu_handler.rs`** - Handler de menu
- **`notifications.rs`** - Notificações UI
- **`panels.rs`** - Gerenciamento de painéis

**Componentes** (`src/ui/components/`):
- **`mod.rs`** - Re-exports
- **`gif_manager.rs`** - Gerenciador de GIFs animados
- **`item_slot/mod.rs`** - Slot de item para grid
- **`media_preview.rs`** - Preview genérico de mídia
- **`mpv_preview/mod.rs`** - Preview de vídeo com mpv
- **`video_controls_state.rs`** - Estado de controles de vídeo
- **`video_menu.rs`** - Menu de vídeo
- **`virtual_drive_settings.rs`** - Configuração de drives virtuais

**Submódulos de componentes refatorados**:
- **`item_slot/`** - Renderização de slots por tipo
  - **`mod.rs`** - Dispatch principal
  - **`drive_slot.rs`** - Slot de drive
  - **`folder_slot.rs`** - Slot de pasta
  - **`file_slot.rs`** - Slot de arquivo
  - **`badges.rs`** - Badges de sync/status
- **`mpv_preview/`** - Bridge MPV por responsabilidade
  - **`mod.rs`** - Coordenador do preview
  - **`lifecycle.rs`** - Inicialização/shutdown
  - **`playback_state.rs`** - Estado e comandos de playback
  - **`docked_filters.rs`** - Filtros/perfil docked
  - **`osc_input.rs`** - Sync de OSC e forwarding de input
  - **`update_loop.rs`** - Loop principal de update/render do preview
  - **`window_embed.rs`** - Embedding e sync da janela nativa

**MPV Sub-módulo** (`src/ui/components/mpv/`):
- **`mod.rs`** - Re-exports
- **`event_loop.rs`** - Event loop do mpv
- **`filters.rs`** - Filtros de vídeo
- **`playback.rs`** - Controle de playback
- **`state.rs`** - Estado do player
- **`utils.rs`** - Utilitários

**Views** (`src/ui/views/`):
- **`mod.rs`** - Re-exports
- **`common.rs`** - Funções comuns às views
- **`computer_view.rs`** - View "Este Computador"
- **`grid_view/mod.rs`** - Visualização em grade
- **`list_view/`** - Visualização em lista (sub-módulo)
  - **`mod.rs`** - Entry point
  - **`header.rs`** - Cabeçalho da lista
  - **`helpers.rs`** - Funções auxiliares
  - **`item_renderer.rs`** - Renderização de itens
  - **`virtualization.rs`** - Virtualização de lista

**Submódulo de grid view refatorado** (`src/ui/views/grid_view/`):
- **`mod.rs`** - Orquestração principal da grid
- **`virtualization.rs`** - Cálculo de janela visível
- **`item_renderer.rs`** - Renderização dos itens
- **`scroll.rs`** - Gerenciamento de scroll
- **`prefetch.rs`** - Estratégia de prefetch
- **`interactions.rs`** - Interações e eventos de input

**Preview Panel** (`src/ui/preview_panel/`):
- **`mod.rs`** - Entry point do preview panel
- **`actions.rs`** - Ações do preview
- **`fallback_renderer.rs`** - Renderização de fallback
- **`file_info_table.rs`** - Tabela de informações
- **`image_preview.rs`** - Preview de imagens
- **`utils.rs`** - Utilitários
- **`video_preview/`** - Preview de vídeo
  - **`mod.rs`** - Coordenação
  - **`controls.rs`** - Fachada de controles
  - **`controls/pickers.rs`** - Pickers de audio/legenda
  - **`controls/detached.rs`** - Controles exclusivos do modo destacado
  - **`detached.rs`** - Janela destacada
  - **`docked.rs`** - Painel docked
  - **`fullscreen.rs`** - Tela cheia

**Arquivos principais**:
- **`app_impl.rs`** - Implementação eframe::App
- **`cache.rs`** - CacheManager para texturas
- **`context_menu.rs`** - Menu de contexto UI
- **`icon_loader.rs`** - Fachada do loader de ícones
- **`icon_loader/async_ops.rs`** - Carga assíncrona e integração com workers
- **`icon_loader/file_icons.rs`** - Carregamento de ícones de arquivo
- **`icon_loader/special_icons.rs`** - Ícones especiais e shortcuts
- **`navigation.rs`** - Navegação UI
- **`sidebar.rs`** - Sidebar com atalhos, Acesso Rápido (pastas fixadas, drag-and-drop, scroll)
- **`status_bar.rs`** - Barra de status
- **`svg_icons.rs`** - Gerenciador de ícones SVG
- **`tab_bar/mod.rs`** - Sistema de abas
- **`theme.rs`** - Tema e cores
- **`toolbar.rs`** - Barra de ferramentas
- **`widgets.rs`** - Widgets customizados
- **`global_search_overlay.rs`** - Fachada do overlay modal de busca global (Ctrl+Shift+F)
- **`global_search_overlay/filters.rs`** - Filtros de drive/tipo e parsing de query
- **`global_search_overlay/results_panel.rs`** - Renderização e ativação de resultados

**Submódulo de tab bar refatorado** (`src/ui/tab_bar/`):
- **`mod.rs`** - Coordenador principal
- **`tabs_renderer.rs`** - Renderização de tabs
- **`window_controls.rs`** - Controles de janela
- **`drag_dwell.rs`** - Lógica de hover/drag dwell
- **`new_tab_area.rs`** - Área de criação de nova aba

---

### 6. `src/workers/` - Threads de Background
**Propósito**: Processamento assíncrono para manter UI responsiva

**Thumbnail System** (`src/workers/thumbnail/`):
- **`mod.rs`** - Coordenação do sistema de thumbnails
- **`extraction/`** - Extraction stages
  - **`mod.rs`** - Re-exports
  - **`stage1_image_crate.rs`** - Stage 1: image crate (PNG, JPG, GIF, WebP)
  - **`stage2_wic.rs`** - Stage 2: Windows Imaging Component
  - **`stage3_shell_api.rs`** - Stage 3: Shell API (IShellItemImageFactory)
  - **`stage4_force_extract.rs`** - Stage 4: Extração forçada
  - **`stage5_media_foundation.rs`** - Stage 5: Media Foundation

**Workers individuais**:
- **`batch_thumbnail_loader.rs`** - Loader em batch (não usado atualmente)
- **`file_operation_worker.rs`** - Operações de arquivo (copiar, mover, deletar)
- **`folder_preview_worker.rs`** - Geração de previews de pastas
- **`folder_scanner.rs`** - Scanner de pastas
- **`global_search_worker.rs`** - Worker de busca global (IPC com mtt-search-service)
- **`idle_warmup.rs`** - Warmup de cache em idle
- **`predictive_prefetch.rs`** - Prefetch preditivo
- **`prefetch_worker.rs`** - Worker de pré-carregamento
- **`thumbnail_loader.rs`** - Loader de thumbnails
- **`mod.rs`** - Re-exports

---

### 7. `src/tabs/` - Sistema de Abas
**Propósito**: Gerenciamento de múltiplas abas

**Arquivos**:
- **`mod.rs`** - Implementação do sistema de abas

**Funcionalidades**:
- Histório independente por aba
- Estado de view por aba
- Preview independente por aba

---

### 8. `src/pdf_viewer/` - Visualizador de PDF
**Propósito**: Visualização de PDFs via WebView2

**Arquivos**:
- **`mod.rs`** - Re-exports
- **`thread.rs`** - Thread dedicada para WebView2
- **`webview.rs`** - Interface com WebView2
- **`window.rs`** - Gerenciamento de janela PDF

---

### 9. `crates/mtt-search-protocol/` - Protocolo IPC
**Propósito**: Tipos compartilhados entre app e serviço de busca para comunicação via Named Pipes

**Arquivos**:
- **`src/lib.rs`** - Definições de tipos e funções de serialização

**Principais structs/enums**:
```rust
pub const PIPE_NAME: &str = r"\\.\pipe\MTTFileManagerSearch";

pub enum SearchRequest { Query { text, offset, limit }, GetStatus, Ping, WarmIndex }
pub enum SearchResponse { Results { items, has_more, total_matches }, Status(IndexStatusInfo), Pong, WarmStarted, Error(String) }
pub struct SearchResultItem { name, full_path, is_dir, size }
pub struct IndexStatusInfo { volumes: Vec<VolumeStatus>, total_files_indexed }
pub struct VolumeStatus { drive_letter, state, files_indexed }

pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, String>  // 4-byte LE prefix + bincode
pub fn decode_message<T: Deserialize>(data: &[u8]) -> Result<T, String>
```

**Dependências**: `serde`, `bincode`

---

### 10. `crates/mtt-search-service/` - Serviço de Busca
**Propósito**: Windows Service que indexa todos os arquivos com estratégia híbrida por volume (USN + fallback full-scan) e serve buscas via Named Pipes

**Arquivos**:
- **`main.rs`** - Entry point, command-line dispatch (`install`, `uninstall`, `run-console`) e orquestração de workers
- **`volume_indexers.rs`** - Indexadores por volume (USN incremental + fallback sem USN)
- **`usn_journal.rs`** - Descoberta de volumes (`discover_volumes`) + API USN (`open_volume`, `query_usn_journal`, `enumerate_all_files`, `read_usn_buffer`, `parse_usn_records`)
- **`fs_walker.rs`** - Varredura full-tree para volumes sem USN (BFS iterativo, ignora reparse points)
- **`file_index.rs`** - Índice in-memory: `VolumeIndex` (HashMap<u64, FileRecord>), `search()` com deadline de 5s
- **`path_resolver.rs`** - `resolve_path(frn, index)` via cadeia de parent references até FRN 5 (funciona para FRN NTFS e sintético)
- **`index_db.rs`** - SQLite em `%PROGRAMDATA%\MTT-File-Manager\search_index.db`: tables `volume_state`, `file_records`
- **`ipc_server.rs`** - Named Pipe server com NULL DACL, overlapped I/O, handlers para Query/GetStatus/Ping/WarmIndex
- **`service_control.rs`** - Install/uninstall via `windows-service` (nome: `MTTFileManagerSearch`, AutoStart, LocalSystem)

**Principais structs**:
```rust
pub struct FileRecord { parent_ref, name_offset, name_len, is_dir, _pad }
pub struct VolumeIndex { drive_letter, records: HashMap<u64, FileRecord>, last_usn, journal_id, state }
pub enum IndexState { NotStarted, Scanning, Ready, Error(String) }
```

**Dependências**: `mtt-search-protocol`, `windows`, `windows-service`, `rusqlite`, `serde`, `bincode`

---

### 11. Arquivos Raiz

**`src/main.rs`**:
- Entry point do binário
- Carrega ícone do aplicativo
- Configura viewport (borderless window)
- Inicializa codec registry
- Chama `eframe::run_native()`

**`src/lib.rs`**:
- Entry point da biblioteca
- Declara todos os módulos públicos
- Re-exporta `ImageViewerApp`

**`src/embedded_assets.rs`**:
- Fontes embarcadas (remixicon.ttf)
- Recursos para executável portátil

## Fluxo de Dados Principal

```
User Input
    ↓
src/ui/app/input.rs
    ↓
src/app/operations/
    ↓
src/application/ (lógica de negócio)
    ↓
src/infrastructure/ (Windows API, I/O)
    ↓
src/workers/ (processamento assíncrono)
    ↓
Channels → src/ui/app_impl.rs (atualização UI)
```

## Dependências entre Módulos

```
main.rs
    ↓
lib.rs
    ├── app/ ────────┬──► application/
    │   │            ├──► domain/
    │   └──► workers/    ├──► infrastructure/
    ├── ui/ ◄────────────┘         │
    ├── tabs/                      └──► mtt-search-protocol (IPC types)
    └── pdf_viewer/

mtt-search-service (processo separado)
    ├──► mtt-search-protocol (IPC types)
    ├──► windows (USN Journal, descoberta de volumes, Named Pipes)
    ├──► rusqlite (persistência)
    └──► windows-service (SCM integration)
```

**Regras de Dependência**:
1. `domain` não depende de nenhum outro módulo local
2. `application` depende apenas de `domain` e `infrastructure`
3. `infrastructure` não depende de `ui` ou `app`
4. `workers` dependem de `infrastructure` e `domain`
5. `app` depende de todos os outros módulos
6. `ui` depende de `app`, `domain`, `application`, `infrastructure`
7. `mtt-search-protocol` não depende de nenhum módulo local (apenas `serde`, `bincode`)
8. `mtt-search-service` depende de `mtt-search-protocol`, `windows`, `rusqlite`, `windows-service`
9. `mtt-file-manager` depende de `mtt-search-protocol` (para comunicação IPC com o serviço)

## Arquivos de Configuração do Projeto

**`Cargo.toml` (raiz)**:
- Workspace com 3 members: `.`, `crates/mtt-search-protocol`, `crates/mtt-search-service`
- Dependências do app principal
- Workspace dependencies compartilhadas: `serde`, `bincode`, `rusqlite`
- Features (notify-watcher)
- Profile de release (LTO, opt-level 3)

**`build.rs`**:
- Configuração de recursos Windows (ícone)

---

*Última atualização: 2026-02-18 (adicionado pinned_folder, pinned_folder_ops, disk_cache/pinned_folders, folder_lock_ops, folder_locks)*
