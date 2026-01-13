# Documentação das Seções do `main.rs`

Este documento mapeia as seções lógicas do arquivo `main.rs` (~5000 linhas) para facilitar a refatoração planejada.

---

## 📋 Índice de Seções

| Seção | Linhas | Responsabilidade |
|-------|--------|------------------|
| [1. Imports & Constants](#1-imports--constants) | 1-91 | Dependências e configuração global |
| [2. Enums & Types](#2-enums--types) | 83-91 | Tipos auxiliares (ClipboardOp) |
| [3. ImageViewerApp Struct](#3-imageviewerapp-struct) | 94-239 | Definição do estado da aplicação |
| [4. Constructor (new)](#4-constructor-new) | 241-585 | Inicialização e spawn de workers |
| [5. UI Helper Methods](#5-ui-helper-methods) | 587-870 | Botões de ícone, toggles |
| [6. Clipboard Operations](#6-clipboard-operations) | 773-890 | Copiar, Recortar, Colar |
| [7. Filtering & Sorting](#7-filtering--sorting) | 892-1020 | filter_items, sort_items |
| [8. Preferences & Persistence](#8-preferences--persistence) | 1022-1110 | save_preferences, load from SQLite |
| [9. Folder Loading](#9-folder-loading) | 1112-1380 | load_folder, streaming batch loading |
| [10. Navigation](#10-navigation) | 1382-1620 | navigate_to, go_back, go_forward, go_up |
| [11. Special Views](#11-special-views) | 1450-1620 | Computer View, Recycle Bin View |
| [12. Tab Management](#12-tab-management) | 1570-1660 | sync_to_tab, sync_from_tab |
| [13. File System Watcher](#13-file-system-watcher) | 1770-1830 | watch_current_folder |
| [14. Rename Operations](#14-rename-operations) | 1830-1880 | rename_with_shell |
| [15. Thumbnail & Icon Loading](#15-thumbnail--icon-loading) | 1880-2010 | request_thumbnail_load, get_or_load_icon |
| [16. Metadata Handling](#16-metadata-handling) | 2010-2120 | refresh_selected_metadata, format helpers |
| [17. Message Processing](#17-message-processing) | 2120-2400 | process_incoming_messages |
| [18. List View Rendering](#18-list-view-rendering) | 2245-2540 | render_list_view |
| [19. Grid View Rendering](#19-grid-view-rendering) | 2542-2770 | render_grid_view |
| [20. Item Slot Rendering](#20-item-slot-rendering) | 2771-2920 | render_item_slot |
| [21. Context Menu](#21-context-menu) | 2970-3195 | populate_context_menu |
| [22. eframe::App Implementation](#22-eframeapp-implementation) | 3197-4820 | update(), on_exit() |
| [23. Main Function](#23-main-function) | 4840-4957 | Ponto de entrada, fonts, viewport |

---

## 1. Imports & Constants
**Linhas: 1-91**

```rust
// Estrutura:
// - use statements (eframe, lru, notify, rayon, std, windows)
// - Remix Icon constants (ICON_ARROW_LEFT, etc.)
// - Domain/Infrastructure imports
// - Windows API imports
// - Global constants (PATH_PADRAO, CACHE_SIZE, etc.)
// - Helper function to_win32_path()
```

**Dependências Críticas:**
- `eframe::egui` - Framework GUI
- `lru::LruCache` - Cache de texturas
- `notify` - File system watcher
- `rayon` - Parallel sorting
- `windows` crate - Win32 APIs

**Constantes Importantes:**
- `CACHE_SIZE = 200` - Limite de texturas em memória
- `ICON_CACHE_SIZE = 100` - Cache de ícones
- `DRIVE_REFRESH_INTERVAL_MS = 350` - Polling de drives

---

## 2. Enums & Types
**Linhas: 83-91**

```rust
#[derive(Clone, Copy, PartialEq, Debug)]
enum ClipboardOp {
    Copy,
    Move,
}
```

**Nota:** A maioria dos tipos está em `domain/` (FileEntry, SortMode, ViewMode, etc.)

---

## 3. ImageViewerApp Struct
**Linhas: 94-239**

A struct principal contém ~50 campos organizados em categorias:

### 3.1 Core State
- `current_path: String` - Caminho atual
- `items: Arc<Vec<FileEntry>>` - Lista de arquivos (Arc para clone barato)
- `all_items: Vec<FileEntry>` - Cache mestre para busca

### 3.2 Worker Channels
- `thumbnail_req_sender/image_receiver` - Pool de thumbnails
- `cover_worker_sender/receiver` - Capas de pastas
- `folder_preview_sender/receiver` - Previews nativos Windows
- `metadata_req_sender/res_receiver` - Metadados de mídia
- `icon_req_sender/res_receiver` - Ícones assíncronos
- `folder_size_req_sender/res_receiver` - Cálculo de tamanho

### 3.3 Async Loading
- `file_entry_sender/receiver` - Streaming de FileEntry
- `is_loading_folder: bool` - Flag de carregamento

### 3.4 Cache Manager
- `cache_manager: CacheManager` - Unifica texture_cache, icon_cache, loading_set

### 3.5 Sorting & View
- `sort_mode: SortMode`
- `sort_descending: bool`
- `folders_position: FoldersPosition`
- `view_mode: ViewMode`
- `thumbnail_size: f32`

### 3.6 Navigation
- `navigation_history: Vec<String>`
- `history_index: usize`
- `path_input: String`
- `is_address_editing: bool`

### 3.7 Selection
- `selected_item: Option<usize>`
- `selected_file: Option<FileEntry>`
- `selected_thumbnail: Option<TextureHandle>`
- `selected_metadata: Option<(PathBuf, MediaMetadata)>`

### 3.8 Special Views
- `is_computer_view: bool`
- `is_recycle_bin_view: bool`
- `disks: Vec<(String, String)>`

### 3.9 UI State
- `renaming_state: Option<(usize, String)>`
- `focus_rename: bool`
- `context_menu: ContextMenuState`
- `notifications: NotificationManager`
- `show_preview_panel: bool`

### 3.10 File System
- `watcher: Option<RecommendedWatcher>`
- `fs_event_sender/receiver`
- `pending_auto_reload: bool`

### 3.11 Clipboard
- `clipboard_file: Option<PathBuf>`
- `clipboard_op: Option<ClipboardOp>`

### 3.12 Window State
- `native_hwnd: Option<HWND>`
- `startup_tick: usize`
- `saved_window_width/height`
- `saved_is_maximized`
- `sidebar_left/right_width`

### 3.13 Tab System
- `tab_manager: TabManager`

---

## 4. Constructor (new)
**Linhas: 241-585**

### Responsabilidades:
1. Criar canais mpsc para todos os workers
2. Spawn workers em threads separadas:
   - Cover Worker (capas de pastas)
   - Thumbnail Worker Pool (4+ threads via rayon)
   - Icon Worker (single thread)
   - Metadata Worker (single thread)
   - Folder Preview Worker
   - Folder Size Worker
3. Inicializar OneDrive paths
4. Carregar preferências do SQLite (disk_cache)
5. Configurar watcher inicial
6. Iniciar Garbage Collector em background

### Workers Spawned:
```
┌─────────────────────────────────────────────────────────────┐
│                        MAIN THREAD                          │
│                     (ImageViewerApp)                        │
└─────────────────────────────────────────────────────────────┘
         │              │              │              │
         ▼              ▼              ▼              ▼
   ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐
   │ Cover    │  │ Thumbnail│  │  Icon    │  │ Metadata │
   │ Worker   │  │ Pool(4+) │  │ Worker   │  │ Worker   │
   └──────────┘  └──────────┘  └──────────┘  └──────────┘
```

---

## 5. UI Helper Methods
**Linhas: 587-870**

### Métodos:
- `icon_button()` - Renderiza botão com ícone SVG
- `toggle_icon_button()` - Botão toggle (ativo/inativo)
- `delete_with_shell_for_idx()` - Excluir via SHFileOperationW
- `restore_from_recycle_bin()` - Restaurar da lixeira
- `delete_permanently()` - Excluir permanentemente
- `empty_recycle_bin()` - Esvaziar lixeira
- `show_properties_for_idx()` - Diálogo de propriedades
- `create_new_folder()` - Criar nova pasta

---

## 6. Clipboard Operations
**Linhas: 773-890**

### Métodos:
- `command_copy(idx)` - Ctrl+C
- `command_cut(idx)` - Ctrl+X  
- `command_paste(idx)` - Ctrl+V

**Implementação:**
- Usa Windows Clipboard API (CF_HDROP format)
- Fallback para clipboard interno
- SHFileOperationW para operações de arquivo

---

## 7. Filtering & Sorting
**Linhas: 892-1020**

### Métodos:
- `filter_items()` - Filtra por search_query
- `sort_items()` - Ordena usando natord (natural sorting)

**Características:**
- Threshold adaptativo: parallel sort para >5000 itens
- Suporta pastas primeiro/último/misturado
- Ordena por Nome, Data, Tamanho, Tipo

---

## 8. Preferences & Persistence
**Linhas: 1022-1110**

### Método Principal:
- `save_preferences()` - Salva no SQLite via disk_cache

**Preferências Salvas:**
- sort_mode, sort_descending, folders_position
- thumbnail_size, view_mode, show_preview_panel
- window_width, window_height, window_is_maximized
- sidebar_left_width, sidebar_right_width

---

## 9. Folder Loading
**Linhas: 1112-1380**

### Método Principal:
- `load_folder(force_refresh: bool)`

**Características:**
- Generation tracking para cancelamento
- Streaming batch loading (250 itens por lote)
- Win32 FindFirstFileW/FindNextFileW (uma syscall por arquivo)
- OneDrive sync status detection
- Debounce de auto-reload via watcher

---

## 10. Navigation
**Linhas: 1382-1620**

### Métodos:
- `navigate_to(path)` - Navega para path (adiciona ao histórico)
- `go_back()` - Volta no histórico
- `go_forward()` - Avança no histórico
- `go_up_one_level()` - Sobe para pasta pai
- `can_go_back()` / `can_go_forward()` - Checks de navegação

**Características:**
- Histórico linear com truncamento ao navegar
- Normalização de drive roots (Z: → Z:\)
- Invalidação de folder previews ao navegar

---

## 11. Special Views
**Linhas: 1450-1620**

### Métodos:
- `navigate_to_computer()` - Vista "Este Computador"
- `navigate_to_recycle_bin()` - Vista da Lixeira
- `setup_computer_view()` - Popula drives
- `setup_recycle_bin_view()` - Enumera lixeira (async)
- `reload_drive_list()` - Atualiza lista de drives
- `refresh_drives_if_needed()` - Polling periódico

---

## 12. Tab Management
**Linhas: 1570-1660**

### Métodos:
- `sync_to_tab()` - Salva estado no tab atual
- `sync_from_tab()` - Restaura estado do tab

**Estado Sincronizado:**
- path, path_input, is_computer_view
- navigation_history, history_index
- items, all_items, selected_item, selected_file
- search_query, scroll_to_selected

---

## 13. File System Watcher
**Linhas: 1770-1830**

### Método:
- `watch_current_folder()` - Configura notify watcher

**Eventos Tratados:**
- Remove → Limpa cache
- Modify → Invalida folder previews
- Debounce de 500ms para auto-reload

---

## 14. Rename Operations
**Linhas: 1830-1880**

### Método:
- `rename_with_shell(idx)` - Renomeia via SHFileOperationW

**Características:**
- Suporta Undo (Ctrl+Z)
- FOF_ALLOWUNDO flag

---

## 15. Thumbnail & Icon Loading
**Linhas: 1880-2010**

### Métodos:
- `request_thumbnail_load(path)` - Envia para worker pool
- `request_folder_preview_load(path)` - Preview nativo Windows
- `get_or_load_icon(ctx, path)` - Cache por extensão ou path
- `ensure_folder_icon(ctx)` - Garante ícone de pasta
- `ensure_computer_icon(ctx)` - Garante ícone de computador

---

## 16. Metadata Handling
**Linhas: 2010-2120**

### Métodos:
- `refresh_selected_metadata()` - Carrega metadata async
- `format_media_duration(ticks)` - Formata duração
- `format_bitrate(bps)` - Formata bitrate
- `approximate_bitrate(size, duration)` - Calcula bitrate

---

## 17. Message Processing
**Linhas: 2120-2400**

### Método Principal:
- `process_incoming_messages(ctx)`

**Processa:**
1. F5 → Manual refresh
2. Device events → Reload drives
3. FS watcher events → Auto-reload
4. FileEntry batches → Streaming loading
5. Cover worker results → Folder covers
6. Icon worker results → Async icons
7. Metadata worker results → Media info
8. Thumbnail results → Texture cache
9. Folder preview results → Native previews
10. Folder size results → Size calculation

---

## 18. List View Rendering
**Linhas: 2245-2540**

### Método:
- `render_list_view(ui)`

**Delega para:** `mtt_file_manager::ui::views::list_view`

**Responsabilidades:**
- Keyboard navigation (↑↓, Enter)
- Click/DoubleClick/SecondaryClick handling
- Sort header clicks
- Renaming inline

---

## 19. Grid View Rendering
**Linhas: 2542-2770**

### Método:
- `render_grid_view(ui)`

**Delega para:** `mtt_file_manager::ui::views::grid_view`

**Responsabilidades:**
- Keyboard navigation (←→↑↓, Enter)
- Click/DoubleClick/SecondaryClick handling
- Thumbnail loading on visibility
- Folder preview loading

---

## 20. Item Slot Rendering
**Linhas: 2771-2920**

### Método:
- `render_item_slot(ui, idx)` [UNUSED - dead code]

**Delega para:** `mtt_file_manager::ui::components::item_slot`

---

## 21. Context Menu
**Linhas: 2970-3195**

### Métodos:
- `populate_context_menu(ctx, path, is_empty_area, idx)`
- `context_target_path(idx)` - Resolve target
- `copy_path_to_clipboard(path)` - Copia path como texto
- `create_shell_shortcut(target)` - Cria .lnk

**Características:**
- Items primários (header com ícones)
- Items secundários (app-specific)
- Shell extensions (7-Zip, WinRAR, etc.)
- Overflow menu para items extras

---

## 22. eframe::App Implementation
**Linhas: 3197-4820**

### Método Principal:
- `update(ctx, frame)` - Loop principal de UI

### Estrutura do Update:
```
1. Startup sequence (3-stage: hidden → resize → reveal)
2. Window state tracking
3. Ensure window handle (HWND)
4. Keyboard shortcuts (Ctrl+C/X/V, Delete, Ctrl+Shift+N)
5. Tab shortcuts (Ctrl+T/W, Ctrl+Tab)
6. process_incoming_messages()
7. refresh_drives_if_needed()
8. ensure_folder_icon() / ensure_computer_icon()

=== PANELS ===
9. TopBottomPanel::bottom - Status bar
10. TopBottomPanel::top("tab_bar") - Tab bar + window controls
11. TopBottomPanel::top("nav_bar") - Navigation + address bar
12. SidePanel::left("sidebar") - Drives + shortcuts
13. SidePanel::right("preview_panel") - Details pane
14. CentralPanel - Grid/List view
15. Context menu overlay
16. Resize grip overlay
17. Toast notifications overlay
```

### on_exit():
- Salva preferências antes de fechar

---

## 23. Main Function
**Linhas: 4840-4957**

### Responsabilidades:
1. Inicializa codec cache
2. Carrega ícone da aplicação (appicon.png)
3. Configura ViewportBuilder (hidden, 800x600)
4. Configura fonts (Segoe UI, Remix Icon)
5. Executa `eframe::run_native()`

---

## 🎯 Prioridades de Refatoração

### Alta Prioridade (Fase 1-2):
1. **Extrair ImageViewerApp struct** → `app/state.rs`
2. **Extrair constructor** → `app/init.rs`
3. **Extrair message processing** → `app/messages.rs`

### Média Prioridade (Fase 3-4):
4. **Extrair navigation** → `app/navigation.rs`
5. **Extrair clipboard ops** → `app/clipboard.rs`
6. **Extrair special views** → `app/views.rs`

### Baixa Prioridade (Fase 5-6):
7. **Extrair context menu** → `app/context_menu.rs`
8. **Extrair update() panels** → `ui/panels/*.rs`
9. **Extrair main()** → manter minimal

---

## 📊 Métricas

| Métrica | Valor |
|---------|-------|
| Total de linhas | ~4957 |
| Métodos impl ImageViewerApp | ~50 |
| Workers spawned | 6 |
| Canais mpsc | 12 pairs |
| Panels egui | 5 |
| Overlays | 3 |

---

*Documento gerado para suportar a refatoração conforme REFACTORING_PLAN.md*
