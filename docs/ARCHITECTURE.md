# Arquitetura do Sistema - MTT File Manager

## 1. Tipo de Arquitetura

**Arquitetura em Camadas (Layered Architecture)** com separação clara de responsabilidades:

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Layer                             │
│   eframe App impl, components, views, preview panels        │
├─────────────────────────────────────────────────────────────┤
│                   Application Layer                         │
│   Services: clipboard, navigation, sorting, file_ops        │
├─────────────────────────────────────────────────────────────┤
│                      App Layer                              │
│   State container, operations, message handlers             │
├─────────────────────────────────────────────────────────────┤
│                     Domain Layer                            │
│   FileEntry, DriveInfo, SortMode, ViewMode, Errors          │
├─────────────────────────────────────────────────────────────┤
│                 Infrastructure Layer                        │
│   Windows API, media extraction, caching, security          │
├─────────────────────────────────────────────────────────────┤
│                    Workers Layer                            │
│   Background threads: thumbnails, folder preview            │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. Camadas Detalhadas

### 2.1 UI Layer (`src/ui/`)

**Responsabilidade:** Renderização de interface e interação com usuário

**Componentes principais:**
- `app_impl.rs` - Implementação do trait `eframe::App`
- `preview_panel.rs` - Painel de visualização (58KB - maior arquivo)
- `sidebar.rs` - Sidebar com árvore de pastas
- `toolbar.rs` - Barra de ferramentas
- `tab_bar.rs` - Gerenciamento visual de abas
- `context_menu.rs` - Menus de contexto customizados
- `views/` - Grid e List views para arquivos

**Sub-componentes:**
- `components/mpv_preview.rs` - Player de vídeo embutido
- `components/media_preview.rs` - Preview de mídia (imagens, GIFs)
- `components/item_slot.rs` - Célula individual de arquivo/pasta

### 2.2 Application Layer (`src/application/`)

**Responsabilidade:** Lógica de negócio e serviços reutilizáveis

**Módulos:**
- `clipboard.rs` - Operações de clipboard (copy/paste de arquivos)
- `navigation.rs` - Histórico de navegação (back/forward)
- `sorting.rs` - Ordenação de arquivos (nome, data, tamanho, tipo)
- `file_operations.rs` - Operações de arquivo (delete, rename, move)
- `context_menu.rs` - Lógica de menu de contexto
- `notification.rs` - Sistema de notificações
- `state.rs` - Estado compartilhado entre componentes

### 2.3 App Layer (`src/app/`)

**Responsabilidade:** Container de estado e orquestração

**Estrutura:**
- `state.rs` - Struct `ImageViewerApp` (estado global da aplicação)
- `init.rs` - Inicialização da aplicação (430 linhas)
- `operations/` - 19 módulos de operações:
  - `folder_loading.rs` - Carregamento de pastas
  - `navigation.rs` - Handlers de navegação
  - `selection.rs` - Lógica de seleção
  - `message_handler.rs` - Processamento de mensagens assíncronas
  - `ui_rendering.rs` - Orquestração de renderização
  - `recycle_bin_ops.rs` - Operações de lixeira
  - E outros 13 módulos

### 2.4 Domain Layer (`src/domain/`)

**Responsabilidade:** Modelos de dados e tipos centrais

**Tipos:**
- `FileEntry` - Representação de arquivo/pasta com metadados
- `DriveInfo` - Informações de volume/drive
- `SortMode` - Enum de modos de ordenação
- `ViewMode` - Grid ou List
- `IconSize` - Small, Large, Jumbo
- `SyncStatus` - Status OneDrive
- `AppError` - Hierarquia de erros
- `AppResult<T>` - Tipo Result padrão

### 2.5 Infrastructure Layer (`src/infrastructure/`)

**Responsabilidade:** Integração com sistema operacional e serviços externos

**Módulos Windows (`windows/`):**
- `shell_operations.rs` - SHFileOperation, context menus
- `recycle_bin.rs` - IShellItem2, lixeira
- `icons.rs` - Extração de ícones do sistema
- `native_menu.rs` - Menus shell nativos
- `media_foundation.rs` - Extração de frames via MF
- `codec_registry.rs` - Registry de codecs de vídeo
- `bitmap_conversion.rs` - HBITMAP → RGBA
- `window_subclass.rs` - Subclassing para borderless window

**Outros:**
- `disk_cache.rs` - Cache SQLite de thumbnails
- `security.rs` - Verificações de segurança de caminho
- `onedrive.rs` - Detecção de status OneDrive

### 2.6 Workers Layer (`src/workers/`)

**Responsabilidade:** Processamento em background

**Workers:**
- `thumbnail_worker.rs` (925 linhas) - Pipeline híbrido de thumbnails:
  1. `image` crate (rápido)
  2. WIC (robusto, CMYK)
  3. Media Foundation (vídeos)
  4. Shell API (universal)
- `batch_thumbnail_loader.rs` - Carregamento em lote
- `folder_preview_worker.rs` - Preview de conteúdo de pastas
- `file_operation_worker.rs` - Operações de arquivo assíncronas

---

## 3. Fluxo de Dados

### 3.1 Fluxo de Navegação
```
User Click → UI Layer
    ↓
TabManager.navigate_to()
    ↓
App Layer: folder_loading.rs
    ↓
Infrastructure: walkdir + Windows Shell API
    ↓
Domain: Vec<FileEntry>
    ↓
UI Layer: grid_view/list_view render
```

### 3.2 Fluxo de Thumbnail
```
Grid View detecta item visível
    ↓
PriorityThumbnailQueue.push()
    ↓
Workers: thumbnail_worker_loop
    ↓
Pipeline híbrido (image → WIC → MF → Shell)
    ↓
mpsc::Sender<ThumbnailData>
    ↓
App Layer: message_handler receives
    ↓
UI Layer: texture uploaded to GPU
```

### 3.3 Fluxo de Operação de Arquivo
```
Context Menu → Application Layer
    ↓
Infrastructure: shell_operations.rs
    ↓
Windows: SHFileOperationW / IFileOperation
    ↓
(Optional) file_operation_worker for async
    ↓
Notification to user
    ↓
Folder refresh
```

---

## 4. Pontos de Entrada

### 4.1 `main.rs`
- Inicializa codec cache
- Carrega ícone do app
- Configura viewport (borderless, decorated=false)
- Carrega fontes (Segoe UI, RemixIcon)
- Cria `ImageViewerApp::new()`
- Executa `eframe::run_native()`

### 4.2 `ImageViewerApp::new()` (`app/init.rs`)
- Cria workers de thumbnail
- Inicializa file watcher
- Carrega cache de disco
- Cria TabManager com tab inicial
- Configura channels de comunicação

### 4.3 `ImageViewerApp::update()` (`ui/app_impl.rs`)
- Processa eventos de input
- Processa mensagens de workers
- Renderiza layers (tab_bar, toolbar, main panel, status_bar)

---

## 5. Componentes Centrais

| Componente | Localização | Responsabilidade |
|------------|-------------|------------------|
| `ImageViewerApp` | `app/state.rs` | Container de estado global |
| `TabManager` | `tabs/mod.rs` | Gerenciamento de abas |
| `TabState` | `tabs/mod.rs` | Estado individual de cada aba |
| `FileEntry` | `domain/file_entry.rs` | Modelo de arquivo/pasta |
| `PriorityThumbnailQueue` | `workers/thumbnail_worker.rs` | Fila de requisições de thumbnail |
| `ThumbnailDiskCache` | `infrastructure/disk_cache.rs` | Cache SQLite |
| `MpvPreview` | `ui/components/mpv_preview.rs` | Player de vídeo |
| `SvgIconManager` | `ui/svg_icons.rs` | Renderização de ícones SVG |

---

## 6. Dependências entre Módulos

```
main.rs
  └── lib.rs
        ├── app/
        │     ├── state.rs → domain/, infrastructure/, workers/, ui/
        │     └── init.rs → all modules
        ├── application/ → domain/, infrastructure/
        ├── domain/ → (standalone)
        ├── infrastructure/ → windows crate, domain/
        ├── ui/ → app/, domain/, infrastructure/, application/
        ├── workers/ → infrastructure/, domain/
        ├── tabs/ → application/
        └── pdf_viewer/ → infrastructure/windows
```

---

## 7. Padrões Arquiteturais Identificados

1. **Immediate Mode GUI** - eframe/egui pattern
2. **Message Passing** - mpsc channels entre workers e UI
3. **RAII** - Cleanup automático via Drop (COM, handles)
4. **Pipeline Pattern** - Thumbnail extraction multi-stage
5. **LRU Caching** - Memória e disco
6. **Pub/Sub** - File watcher notifications
7. **State Machine** - Tab navigation history

---

## 8. Pontos de Acoplamento

### Alto Acoplamento:
- `ImageViewerApp` possui ~50 campos, conectado a quase todos os módulos
- `preview_panel.rs` com 58KB depende de muitos componentes

### Baixo Acoplamento:
- `domain/` é independente
- `tabs/` depende apenas de `application/navigation`
- Workers se comunicam apenas via channels
