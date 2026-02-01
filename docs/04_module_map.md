# Mapa do Repositório - MTT File Manager

## Objetivo do Documento
Este documento fornece um mapa detalhado do repositório, listando diretórios, responsabilidades e principais componentes de cada módulo.

## Estrutura de Diretórios

```
src/
├── app/                    # Estado e lógica principal da aplicação
├── application/            # Serviços de lógica de negócios
├── domain/                 # Modelos de dados e regras de negócio
├── infrastructure/         # Integrações com sistema e recursos externos
├── pdf_viewer/            # Visualizador de PDF externo
├── tabs/                  # Sistema de abas
├── ui/                    # Interface do usuário
├── workers/               # Threads de background
├── embedded_assets.rs     # Recursos embarcados
├── lib.rs                 # Entry point da lib
└── main.rs                # Entry point do binário
```

## Módulos Detalhados

### 1. `src/app/` - Estado e Lógica Principal
**Propósito**: Gerenciamento de estado global e inicialização da aplicação

**Arquivos principais**:
- **`state.rs`** - Struct `ImageViewerApp` com estado completo da aplicação
- **`init.rs`** - Inicialização e criação de workers/canais
- **`cache_state.rs`** - Gerenciamento de cache
- **`navigation_state.rs`** - Estado de navegação
- **`ui_state.rs`** - Estado da interface
- **`worker_state.rs`** - Estado dos workers

**Operações** (`src/app/operations/`):
- **`clipboard_ops.rs`** - Operações de clipboard
- **`context_menu.rs`** - Menu de contexto
- **`file_ops.rs`** - Operações de arquivo
- **`folder_loading.rs`** - Carregamento de pastas
- **`icons.rs`** - Gerenciamento de ícones
- **`message_handler.rs`** - Handler de mensagens
- **`metadata.rs`** - Metadados de arquivos
- **`navigation.rs`** - Navegação
- **`preferences.rs`** - Preferências
- **`recycle_bin_ops.rs`** - Operações da lixeira
- **`selection.rs`** - Seleção de arquivos
- **`tabs.rs`** - Gerenciamento de abas
- **`thumbnails.rs`** - Thumbnails
- **`ui_rendering.rs`** - Renderização UI
- **`view_setup.rs`** - Configuração de views
- **`watcher.rs`** - Monitoramento de mudanças
- **`window.rs`** - Gerenciamento de janela

**Principais structs/enums**:
```rust
pub struct ImageViewerApp { /* estado completo */ }
pub enum LastInput { Mouse, Keyboard }
pub enum ScrollRequest { None, EnsureVisible(usize) }
```

**Dependências**: Todos os outros módulos

---

### 2. `src/application/` - Serviços de Negócio
**Propósito**: Lógica de negócios e serviços da aplicação

**Arquivos principais**:
- **`clipboard.rs`** - Gerenciamento de clipboard
- **`context_menu.rs`** - Menu de contexto
- **`file_operations.rs`** - Operações de arquivo
- **`navigation.rs`** - Histórico de navegação
- **`notification.rs`** - Sistema de notificações
- **`renaming.rs`** - Renomeação de arquivos
- **`sorting.rs`** - Ordenação básica
- **`sorting_optimized.rs`** - Ordenação otimizada
- **`state.rs`** - Estado da aplicação
- **`watcher.rs`** - Monitoramento

**Principais structs**:
```rust
pub struct NavigationHistory { /* histórico */ }
pub struct ClipboardManager { /* clipboard */ }
pub struct NotificationManager { /* notificações */ }
```

**Dependências**: `domain`, `infrastructure`

---

### 3. `src/domain/` - Modelos de Dados
**Propósito**: Modelos de dados e regras de negócio centrais

**Arquivos principais**:
- **`errors.rs`** - Tipos de erro da aplicação
- **`file_entry.rs`** - Modelo FileEntry
- **`thumbnail.rs`** - Modelo de thumbnail

**Principais structs/enums**:
```rust
pub struct FileEntry { /* arquivo/diretório */ }
pub struct ThumbnailData { /* thumbnail */ }
pub enum AppError { /* erros */ }
pub enum SortMode { Name, Date, Size, Type }
pub enum ViewMode { Grid, List }
pub enum FoldersPosition { First, Last, Mixed }
```

**Dependências**: Apenas crates externos (std, etc)

---

### 4. `src/infrastructure/` - Integrações Externas
**Propósito**: Acesso a recursos externos e integrações de sistema

**Cache e Storage**:
- **`adaptive_batch.rs`** - Batch adaptativo
- **`cache.rs`** - Cache genérico
- **`cache_first.rs`** - Cache-first strategy
- **`directory_cache.rs`** - Cache de diretórios
- **`directory_index.rs`** - Índice de diretórios
- **`disk_cache.rs`** - Cache em disco (SQLite)
- **`filesystem_cache.rs`** - Cache de filesystem
- **`io_priority.rs`** - Prioridade de I/O
- **`ntfs_reader.rs`** - Leitor NTFS
- **`usn_journal.rs`** - Journal USN do NTFS
- **`virtual_drive_config.rs`** - Config de drives virtuais
- **`watcher.rs`** - Watcher genérico
- **`windows_clipboard.rs`** - Clipboard Windows

**Integrações Windows** (`src/infrastructure/windows/`):
- **`bitmap_conversion.rs`** - Conversão de bitmaps
- **`codec_registry.rs`** - Registro de codecs
- **`device_change.rs`** - Monitoramento de dispositivos
- **`drives.rs`** - Gerenciamento de drives
- **`file_flags.rs`** - Flags de arquivo
- **`file_system.rs`** - Sistema de arquivos
- **`file_type.rs`** - Tipos de arquivo
- **`formatting.rs`** - Formatação
- **`hdd_directory_reader.rs`** - Leitor de diretórios HDD
- **`icons.rs`** - Extração de ícones
- **`iso_mount.rs`** - Montagem de ISO
- **`media_foundation.rs`** - Media Foundation
- **`metadata/`** - Metadados (imagem, vídeo, áudio)
- **`native_menu.rs`** - Menu nativo
- **`recycle_bin.rs`** - Lixeira do Windows
- **`shell_folder.rs`** - Pastas do Shell
- **`shell_operations.rs`** - Operações do Shell
- **`system_info.rs`** - Informações do sistema
- **`window_subclass.rs`** - Subclasse de janela

**Media** (`src/infrastructure/media/`):
- **`ffmpeg_session.rs`** - Sessão FFmpeg
- **`hardware_acceleration.rs`** - Aceleração por hardware
- **`tests_hw.rs`** - Testes de hardware

**Principais funções**:
```rust
pub fn get_all_drives() -> Vec<(String, String)>
pub fn extract_file_icon_by_path() -> Result<(Vec<u8>, u32, u32)>
pub fn extract_media_metadata() -> MediaMetadata
```

**Dependências**: Crates externos (windows, rusqlite, etc)

---

### 5. `src/ui/` - Interface do Usuário
**Propósito**: Componentes de interface e renderização

**App** (`src/ui/app/`):
- **`input.rs`** - Handler de input
- **`lifecycle.rs`** - Ciclo de vida da aplicação
- **`menu_handler.rs`** - Handler de menu
- **`notifications.rs`** - Notificações UI
- **`panels.rs`** - Painéis

**Componentes** (`src/ui/components/`):
- **`gif_manager.rs`** - Gerenciador de GIFs
- **`item_slot.rs`** - Slot de item
- **`media_preview.rs`** - Preview de mídia
- **`mpv_preview.rs`** - Preview MPV
- **`video_menu.rs`** - Menu de vídeo
- **`virtual_drive_settings.rs`** - Config de drives virtuais

**Views** (`src/ui/views/`):
- **`common.rs`** - Funções comuns
- **`computer_view.rs`** - View "Este Computador"
- **`grid_view.rs`** - View em grade
- **`list_view.rs`** - View em lista

**Arquivos principais**:
- **`app_impl.rs`** - Implementação eframe::App
- **`cache.rs`** - Cache UI
- **`context_menu.rs`** - Menu de contexto UI
- **`icon_loader.rs`** - Loader de ícones
- **`navigation.rs`** - Navegação UI
- **`preview_panel.rs`** - Painel de preview
- **`sidebar.rs`** - Sidebar
- **`status_bar.rs`** - Barra de status
- **`svg_icons.rs`** - Ícones SVG
- **`tab_bar.rs`** - Barra de abas
- **`theme.rs`** - Tema da aplicação
- **`toolbar.rs`** - Toolbar
- **`widgets.rs`** - Widgets customizados

**Dependências**: `app`, `application`, `domain`, `infrastructure`

---

### 6. `src/workers/` - Workers de Background
**Propósito**: Processamento assíncrono em threads separadas

**Arquivos principais**:
- **`batch_thumbnail_loader.rs`** - Loader de thumbnails em batch
- **`file_operation_worker.rs`** - Worker de operações de arquivo
- **`folder_preview_worker.rs`** - Worker de preview de pastas
- **`folder_scanner.rs`** - Scanner de pastas
- **`idle_warmup.rs`** - Warmup em idle
- **`predictive_prefetch.rs`** - Prefetch preditivo
- **`prefetch_worker.rs`** - Worker de prefetch
- **`thumbnail_loader.rs`** - Loader de thumbnails
- **`thumbnail_worker.rs`** - Worker de thumbnails
- **`usn_watcher.rs`** - Watcher USN

**Principais structs**:
```rust
pub struct PriorityThumbnailQueue { /* fila de thumbnails */ }
pub struct FileOperationRequest { /* requisição de operação */ }
```

**Dependências**: `infrastructure`, `domain`

---

### 7. `src/tabs/` - Sistema de Abas
**Propósito**: Gerenciamento de múltiplas abas

**Arquivos principais**:
- **`mod.rs`** - Implementação do sistema de abas

**Principais structs**:
```rust
pub struct TabManager { /* gerenciador de abas */ }
pub struct Tab { /* aba individual */ }
```

**Dependências**: `app`, `application`

---

### 8. `src/pdf_viewer/` - Visualizador PDF
**Propósito**: Visualização de PDFs usando WebView2

**Arquivos principais**:
- **`mod.rs`** - Módulo principal
- **`thread.rs`** - Thread do visualizador
- **`webview.rs`** - WebView2 integration
- **`window.rs`** - Janela do visualizador

**Principais funções**:
```rust
pub fn warmup() // Pré-inicialização
pub fn show_pdf_window() // Mostrar janela PDF
```

**Dependências**: Crates externos (webview2-com)

---

## Fluxo de Dados Principal

### 1. Navegação para Pasta
```
User Input → app::operations::navigation → application::navigation → 
infrastructure::windows::file_system → UI Update
```

### 2. Carregamento de Thumbnail
```
UI Request → workers::thumbnail_worker → infrastructure::windows::icons → 
ThumbnailData → app::state → UI Render
```

### 3. Operação de Arquivo
```
User Action → app::operations::file_ops → workers::file_operation_worker → 
Windows Shell → Result → Notification
```

## Pontos de Entrada Principais

### Entry Point do Binário
- **`src/main.rs`** - Função main(), configuração eframe

### Entry Point da Lib
- **`src/lib.rs`** - Re-exports principais

### Entry Points de Workers
- **`workers::thumbnail_worker::spawn_thumbnail_workers()`**
- **`workers::file_operation_worker::start_file_operation_worker()`**

### Entry Points de UI
- **`ui::app_impl::ImageViewerApp::update()`** - Main loop
- **`app::init::ImageViewerApp::new()`** - Inicialização

## Dependências Críticas por Módulo

| Módulo | Dependências Críticas | Descrição |
|--------|----------------------|-----------|
| app | all | Coordena todos os módulos |
| ui | app, application, infrastructure | Renderização e input |
| application | domain, infrastructure | Lógica de negócio |
| infrastructure | windows, rusqlite, image | Integrações externas |
| workers | infrastructure, domain | Processamento assíncrono |
| domain | std, thiserror | Modelos puros |

## Caminho Feliz das Features Críticas

### Navegação de Pasta
1. **`ui::app::input::handle_input()`** - Captura input
2. **`app::operations::navigation::navigate_to_path()`** - Processa navegação
3. **`app::operations::folder_loading::load_folder_contents()`** - Carrega conteúdo
4. **`infrastructure::windows::hdd_directory_reader::read_directory()`** - Lê do disco
5. **`app::operations::thumbnails::request_thumbnails()`** - Solicita thumbnails
6. **`workers::thumbnail_worker`** - Processa thumbnails
7. **`ui::views::grid_view::render()`** - Renderiza resultado

### Preview de Arquivo
1. **`ui::preview_panel::render_preview_panel()`** - Renderiza panel
2. **`app::operations::metadata::request_metadata()`** - Solicita metadados
3. **`infrastructure::windows::metadata::extract_media_metadata()`** - Extrai metadados
4. **`ui::components::media_preview::MediaPreview`** - Renderiza preview
5. **`workers::thumbnail_worker`** - Gera thumbnail se necessário

### Operação de Arquivo
1. **`ui::context_menu::show_context_menu()`** - Mostra menu
2. **`app::operations::file_ops::handle_file_operation()`** - Processa operação
3. **`workers::file_operation_worker::process_operation()`** - Executa em background
4. **`infrastructure::windows::shell_operations`** - Integração Windows
5. **`application::notification::NotificationManager`** - Notifica resultado