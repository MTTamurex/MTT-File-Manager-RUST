# Módulos e Funções - MTT File Manager

## 1. Módulo `app/` - Estado e Operações

### `app/state.rs`

**Responsabilidade:** Container de estado global da aplicação

**Principais Tipos:**
- `ImageViewerApp` - Struct com ~50 campos contendo todo o estado
- `ScrollRequest` - Enum para controle de scroll

**Campos Críticos de ImageViewerApp:**
- `tabs: TabManager` - Gerenciador de abas
- `files: Vec<FileEntry>` - Lista de arquivos da pasta atual
- `selected_item: Option<usize>` - Índice do item selecionado
- `multi_selection: HashSet<usize>` - Seleção múltipla
- `thumbnail_queue: Arc<PriorityThumbnailQueue>` - Fila de thumbnails
- `mpv_preview: Option<MpvPreview>` - Player de vídeo
- `notification: Option<Notification>` - Notificação ativa

---

### `app/init.rs`

**Responsabilidade:** Inicialização da aplicação

**Função Principal:**
```rust
impl ImageViewerApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self
}
```

**Fluxo de Inicialização:**
1. Cria channels de comunicação (mpsc)
2. Inicializa TabManager com tab em "C:\\"
3. Spawna workers de thumbnail
4. Configura file watcher
5. Carrega cache de disco
6. Inicializa SVG icon manager
7. Configura window subclass (borderless)
8. Inicia warmup do WebView2

---

### `app/operations/`

**Responsabilidade:** 19 módulos de operações específicas

| Módulo | Principais Funções |
|--------|-------------------|
| `folder_loading.rs` | `load_folder()`, `load_folder_async()` |
| `navigation.rs` | `go_back()`, `go_forward()`, `go_up_one_level()` |
| `selection.rs` | `handle_selection()`, `toggle_selection()`, `select_all()` |
| `message_handler.rs` | `process_worker_messages()`, `handle_thumbnail_result()` |
| `context_menu.rs` | `show_context_menu()`, `handle_menu_action()` |
| `file_ops.rs` | `delete_selected()`, `create_new_folder()` |
| `recycle_bin_ops.rs` | `restore_from_bin()`, `empty_recycle_bin()` |
| `ui_rendering.rs` | `render_main_panel()`, `render_file_grid()` |

---

## 2. Módulo `application/` - Serviços

### `application/navigation.rs`

**Tipos:**
```rust
pub struct NavigationHistory {
    history: Vec<String>,
    position: usize,
}
```

**Funções:**
- `push(path: &str)` - Adiciona ao histórico
- `go_back() -> Option<String>` - Volta no histórico
- `go_forward() -> Option<String>` - Avança no histórico
- `can_go_back() -> bool`
- `can_go_forward() -> bool`

---

### `application/sorting.rs`

**Funções:**
```rust
pub fn sort_files(
    files: &mut Vec<FileEntry>,
    mode: SortMode,
    ascending: bool,
    folders_position: FoldersPosition
)
```

**Modos de Ordenação:**
- `SortMode::Name` - Ordenação natural (file1, file2, file10)
- `SortMode::Date` - Por data de modificação
- `SortMode::Size` - Por tamanho
- `SortMode::Type` - Por extensão

---

### `application/clipboard.rs`

**Funções:**
```rust
pub fn copy_files_to_clipboard(paths: &[PathBuf]) -> Result<()>
pub fn cut_files_to_clipboard(paths: &[PathBuf]) -> Result<()>
pub fn paste_files_from_clipboard(dest: &Path) -> Result<()>
pub fn get_clipboard_files() -> Option<Vec<PathBuf>>
```

---

## 3. Módulo `domain/` - Modelos

### `domain/file_entry.rs`

**Tipos Principais:**
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

pub enum SortMode { Name, Date, Size, Type }
pub enum ViewMode { Grid, List }
pub enum IconSize { Small, Large, Jumbo }
pub enum SyncStatus { None, CloudOnly, Syncing, Pinned, LocallyAvailable }
```

---

### `domain/errors.rs`

**Tipos:**
```rust
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

pub type AppResult<T> = Result<T, AppError>;
```

**Macros:**
- `safe_unwrap!($expr, $context)` - Unwrap com logging
- `safe_expect!($expr, $message)` - Expect com contexto

---

## 4. Módulo `infrastructure/` - Sistema

### `infrastructure/windows/recycle_bin.rs`

**Funções:**
```rust
pub fn enumerate_recycle_bin() -> Result<Vec<RecycleBinItem>>
pub fn enumerate_recycle_bin_streaming(
    sender: Sender<Vec<RecycleBinItem>>,
    generation: Arc<AtomicUsize>,
    my_gen: usize,
    batch_size: usize
)
pub fn restore_from_recycle_bin(physical: &Path, original: &Path) -> Result<()>
pub fn delete_permanently(path: &Path) -> Result<()>
pub fn empty_recycle_bin() -> Result<()>
pub fn get_recycle_bin_info() -> Result<(u64, u64)>  // (count, size)
```

---

### `infrastructure/windows/shell_operations.rs`

**Funções:**
```rust
pub fn open_with_shell(path: &Path)
pub fn show_shell_context_menu(hwnd: HWND, path: &Path, x: i32, y: i32) -> Result<ContextMenuResult>
pub fn delete_item_with_shell(path: &Path, hwnd: HWND) -> bool
pub fn delete_items_with_shell(paths: &[PathBuf], hwnd: HWND) -> bool
pub fn rename_item_with_shell(path: &Path, new_name: &str, hwnd: HWND) -> bool
pub fn copy_item_with_shell(path: &Path, dest: &Path, hwnd: HWND) -> bool
pub fn move_item_with_shell(path: &Path, dest: &Path, hwnd: HWND) -> bool
pub fn copy_item_with_file_op(path: &Path, dest: &Path, hwnd: HWND) -> bool
```

---

### `infrastructure/windows/icons.rs`

**Funções:**
```rust
pub fn extract_file_icon_by_path(path: &Path, is_dir: bool) -> Option<(Vec<u8>, u32, u32)>
pub fn extract_shell_icon_rgba(path: &Path, is_dir: bool) -> Option<(Vec<u8>, u32, u32)>
pub fn get_fallback_icon() -> (Vec<u8>, u32, u32)
pub fn get_fallback_icon_no_path() -> (Vec<u8>, u32, u32)
```

---

### `infrastructure/disk_cache.rs`

**Tipo:**
```rust
pub struct ThumbnailDiskCache {
    conn: Mutex<Connection>,
    cache_dir: PathBuf,
}
```

**Funções:**
```rust
impl ThumbnailDiskCache {
    pub fn new() -> Result<Self>
    pub fn get(&self, path: &Path, size: u32) -> Option<Vec<u8>>
    pub fn put(&self, path: &Path, size: u32, data: &[u8])
    pub fn invalidate(&self, path: &Path)
    pub fn cleanup_old_entries(&self, max_age_days: i64)
}
```

---

## 5. Módulo `workers/` - Background

### `workers/thumbnail_worker.rs`

**Tipos:**
```rust
pub struct PriorityThumbnailQueue {
    state: Arc<(Mutex<QueueState>, Condvar)>,
}

pub enum ThumbnailPriority {
    High,  // Itens visíveis
    Low,   // Prefetch
}
```

**Funções Principais:**
```rust
pub fn spawn_thumbnail_workers(
    queue: Arc<PriorityThumbnailQueue>,
    tx: Sender<ThumbnailData>,
    ctx: egui::Context,
    gen_tracker: Arc<AtomicUsize>,
    disk_cache: Arc<ThumbnailDiskCache>,
)

fn generate_thumbnail_hybrid(path: &Path) -> Option<(Vec<u8>, u32, u32)>

// Pipeline de 4 estágios:
fn try_image_crate_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)>
fn try_wic_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)>
fn try_media_foundation_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)>
fn extract_windows_thumbnail_shell(path: &Path) -> Result<(Vec<u8>, u32, u32)>
```

---

## 6. Módulo `ui/` - Interface

### `ui/app_impl.rs`

**Implementação:**
```rust
impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame)
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>)
}
```

**Fluxo de Update:**
1. Processa mensagens de workers
2. Renderiza tab_bar (se múltiplas abas)
3. Renderiza toolbar
4. Renderiza main panel (sidebar + file view + preview)
5. Renderiza status bar

---

### `ui/components/mpv_preview.rs`

**Tipo:**
```rust
pub struct MpvPreview {
    mpv: Option<mpv::Mpv>,
    hwnd: Option<HWND>,
    state: Arc<RwLock<MpvState>>,
    current_path: PathBuf,
    // ... outros campos
}
```

**Funções:**
```rust
impl MpvPreview {
    pub fn new(path: PathBuf) -> Self
    pub fn play(&self)
    pub fn pause(&self)
    pub fn toggle_play(&mut self)
    pub fn seek(&self, time: f64)
    pub fn set_volume(&self, volume: f32)
    pub fn set_muted(&self, muted: bool)
    pub fn set_audio_track(&self, id: i64)
    pub fn set_subtitle_track(&self, id: i64)
    pub fn update(&mut self, ui: &mut egui::Ui, frame: Option<&eframe::Frame>)
    pub fn enable_nvidia_vsr(&mut self) -> Result<(), String>
}
```

---

### `ui/views/grid_view.rs`

**Função Principal:**
```rust
pub fn render_grid_view(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
)
```

**Fluxo:**
1. Calcula layout do grid
2. Virtualização: renderiza apenas itens visíveis
3. Para cada item visível:
   - Requisita thumbnail se necessário
   - Renderiza item_slot
   - Processa seleção/hover

---

### `ui/views/list_view.rs`

**Função Principal:**
```rust
pub fn render_list_view(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    ctx: &egui::Context,
)
```

**Colunas:**
- Nome (com ícone)
- Data de modificação
- Tipo
- Tamanho

---

## 7. Módulo `tabs/`

### `tabs/mod.rs`

**Tipos:**
```rust
pub struct TabState {
    pub id: usize,
    pub current_path: String,
    pub history: NavigationHistory,
    pub selected_item: Option<usize>,
    pub multi_selection: HashSet<usize>,
    pub files: Vec<FileEntry>,
    // ... outros campos
}

pub struct TabManager {
    tabs: Vec<TabState>,
    active_index: usize,
    closed_tabs: Vec<TabState>,
    next_id: usize,
}
```

**Funções TabManager:**
```rust
pub fn new() -> Self
pub fn active(&self) -> &TabState
pub fn active_mut(&mut self) -> &mut TabState
pub fn new_tab(&mut self)
pub fn new_tab_at(&mut self, path: &str)
pub fn duplicate_tab(&mut self)
pub fn close_tab(&mut self, index: usize) -> bool  // true = app should close
pub fn switch_to(&mut self, index: usize)
pub fn next_tab(&mut self)
pub fn prev_tab(&mut self)
pub fn reopen_closed_tab(&mut self) -> bool
```

---

## 8. Módulo `pdf_viewer/`

### `pdf_viewer/mod.rs`

**Funções:**
```rust
pub fn open_pdf_viewer(path: PathBuf)  // Fire-and-forget
pub fn open_image_viewer(path: PathBuf)
pub fn warmup()  // Pré-inicializa WebView2
```

### `pdf_viewer/webview.rs`

**Funções:**
```rust
pub fn create_webview(hwnd: HWND, path: &Path) -> Result<ICoreWebView2>
pub fn warmup_env() -> Result<()>
```

---

## Resumo de Responsabilidades

| Camada | Responsabilidade | LOC Estimadas |
|--------|-----------------|---------------|
| `app/` | Estado + Operações | ~3,500 |
| `application/` | Serviços de Negócio | ~1,200 |
| `domain/` | Modelos de Dados | ~400 |
| `infrastructure/` | Sistema Operacional | ~5,000 |
| `ui/` | Interface Gráfica | ~7,000 |
| `workers/` | Background Tasks | ~1,500 |
| `tabs/` | Gerenciamento de Abas | ~350 |
| `pdf_viewer/` | Visualizador Externo | ~500 |
| **Total** | | **~19,450** |
