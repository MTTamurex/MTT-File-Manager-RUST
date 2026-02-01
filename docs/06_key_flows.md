# Fluxos Principais - MTT File Manager

## Objetivo do Documento
Este documento descreve os fluxos principais do aplicativo, incluindo sequência de chamadas, arquivos envolvidos, pontos de bug comuns e como debugar.

## 1. Navegação para Pasta

### Sequência de Chamadas
```
User Input (Click/Enter)
    ↓
ui::app::input::handle_input()
    ↓
app::operations::navigation::navigate_to_path()
    ↓
app::operations::folder_loading::load_folder_contents()
    ↓
infrastructure::windows::hdd_directory_reader::read_directory()
    ↓
[Async] workers::folder_scanner::scan_folder()
    ↓
app::operations::thumbnails::request_thumbnails()
    ↓
workers::thumbnail_worker::process_thumbnail_request()
    ↓
infrastructure::windows::icons::extract_file_icon_by_path()
    ↓
UI Update via channels
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura input do usuário
- **`src/app/operations/navigation.rs`** - Lógica de navegação
- **`src/app/operations/folder_loading.rs`** - Carregamento de pasta
- **`src/infrastructure/windows/hdd_directory_reader.rs`** - Leitura do disco
- **`src/workers/folder_scanner.rs`** - Scanner em background
- **`src/app/operations/thumbnails.rs`** - Solicitação de thumbnails
- **`src/workers/thumbnail_worker.rs`** - Processamento de thumbnails
- **`src/infrastructure/windows/icons.rs`** - Extração de ícones

### Pontos de Bug Comuns
1. **Pasta não carrega**
   - **Causa**: Permissões insuficientes
   - **Debug**: Verificar `AppError::WindowsApi` nos logs
   - **Solução**: Executar como administrador ou verificar ACLs

2. **Performance lenta**
   - **Causa**: Pasta com muitos arquivos (>10k)
   - **Debug**: Verificar `frame_time_avg_ms` no estado
   - **Solução**: Ajustar `upload_budget_ms`, usar virtualização

3. **Thumbnails não aparecem**
   - **Causa**: Codec não suportado, arquivo corrompido
   - **Debug**: Verificar `failed_thumbnails` no estado
   - **Solução**: Verificar `codec_registry::init_codec_cache()`

### Como Debugar
```rust
// Adicionar logs nos pontos críticos
eprintln!("[NAV] Navigating to: {}", path);
eprintln!("[LOAD] Found {} items", items.len());
eprintln!("[THUMB] Requesting thumbnail for: {:?}", path);

// Verificar estado via debugger
println!("is_loading_folder: {}", app.is_loading_folder);
println!("pending_thumbnails: {}", app.pending_thumbnails.len());
```

---

## 2. Preview de Arquivo

### Sequência de Chamadas
```
User Selection (Click)
    ↓
ui::views::grid_view::render_item()
    ↓
app::operations::selection::handle_selection()
    ↓
app::operations::metadata::request_metadata()
    ↓
[Async] infrastructure::windows::metadata::extract_media_metadata()
    ↓
ui::preview_panel::render_preview_panel()
    ↓
ui::components::media_preview::MediaPreview
    ↓
[Video] ui::components::mpv_preview::MpvPreview
[PDF] pdf_viewer::show_pdf_window()
[Image] ui::cache::CacheManager::get_texture()
```

### Arquivos Envolvidos
- **`src/ui/views/grid_view.rs`** - Renderização do item
- **`src/app/operations/selection.rs`** - Handler de seleção
- **`src/app/operations/metadata.rs`** - Solicitação de metadados
- **`src/infrastructure/windows/metadata/mod.rs`** - Extração de metadados
- **`src/ui/preview_panel.rs`** - Painel de preview
- **`src/ui/components/media_preview.rs`** - Preview genérico
- **`src/ui/components/mpv_preview.rs`** - Preview de vídeo
- **`src/pdf_viewer/`** - Preview de PDF

### Pontos de Bug Comuns
1. **Preview não aparece**
   - **Causa**: Arquivo corrompido ou formato não suportado
   - **Debug**: Verificar `selected_metadata` no estado
   - **Solução**: Verificar `extract_media_metadata()` logs

2. **Vídeo não reproduz**
   - **Causa**: libmpv-2.dll não encontrada
   - **Debug**: Verificar console para mensagens de erro mpv
   - **Solução**: Copiar libmpv-2.dll para diretório do executável

3. **PDF não abre**
   - **Causa**: WebView2 Runtime não instalado
   - **Debug**: Verificar `pdf_viewer::warmup()` logs
   - **Solução**: Instalar Microsoft Edge WebView2 Runtime

### Como Debugar
```powershell
# Verificar logs de metadata
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "metadata"

# Verificar se mpv está funcionando
# Testar arquivo diretamente com mpv
mpv.exe arquivo.mp4
```

---

## 3. Operação de Arquivo (Copiar/Mover)

### Sequência de Chamadas
```
User Action (Ctrl+C / Ctrl+V)
    ↓
ui::app::input::handle_input()
    ↓
app::operations::clipboard_ops::handle_copy()
    ↓
app::operations::clipboard_ops::handle_paste()
    ↓
workers::file_operation_worker::process_operation()
    ↓
infrastructure::windows::shell_operations::copy_items()
    ↓
Windows Shell API (IFileOperation)
    ↓
app::operations::notifications::show_notification()
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura atalhos
- **`src/app/operations/clipboard_ops.rs`** - Operações de clipboard
- **`src/workers/file_operation_worker.rs`** - Worker de operações
- **`src/infrastructure/windows/shell_operations.rs`** - Shell API
- **`src/application/notification.rs`** - Notificações

### Pontos de Bug Comuns
1. **Copiar não funciona**
   - **Causa**: Clipboard bloqueado por outro aplicativo
   - **Debug**: Verificar `ClipboardManager` estado
   - **Solução**: Tentar novamente, fechar aplicativos de clipboard

2. **Colar falha**
   - **Causa**: Destino sem permissão de escrita
   - **Debug**: Verificar `FileOperationResult` erro
   - **Solução**: Executar como administrador ou verificar permissões

3. **Operação lenta**
   - **Causa**: Muitos arquivos grandes
   - **Debug**: Verificar progresso em `file_operation_worker`
   - **Solução**: Operação é assíncrona, aguardar conclusão

### Como Debugar
```rust
// Adicionar logs no worker
eprintln!("[FILE_OP] Starting operation: {:?}", operation);
eprintln!("[FILE_OP] Progress: {}/{}", completed, total);

// Verificar clipboard contents
println!("Clipboard has {} items", clipboard.items.len());
```

---

## 4. Geração de Thumbnail

### Sequência de Chamadas
```
Folder Load Complete
    ↓
app::operations::thumbnails::request_thumbnails()
    ↓
workers::thumbnail_worker::spawn_thumbnail_workers()
    ↓
workers::thumbnail_loader::load_thumbnail()
    ↓
[Image] image::load_from_memory() → resize
[Video] infrastructure::windows::media_foundation::extract_video_frame()
[Executable] infrastructure::windows::icons::extract_file_icon_by_path()
    ↓
ui::cache::CacheManager::insert_texture()
    ↓
UI Update
```

### Arquivos Envolvidos
- **`src/app/operations/thumbnails.rs`** - Coordenação de thumbnails
- **`src/workers/thumbnail_worker.rs`** - Workers de thumbnails
- **`src/workers/thumbnail_loader.rs`** - Loader individual
- **`src/infrastructure/windows/media_foundation.rs`** - Vídeo frames
- **`src/infrastructure/windows/icons.rs`** - Ícones de executáveis
- **`src/ui/cache.rs`** - Cache de texturas

### Pontos de Bug Comuns
1. **Thumbnails pretas**
   - **Causa**: Codec de vídeo não suportado
   - **Debug**: Verificar `codec_registry::init_codec_cache()`
   - **Solução**: Verificar Media Foundation está funcionando

2. **Performance de thumbnail lenta**
   - **Causa**: Muitos arquivos grandes
   - **Debug**: Verificar `thumbnail_queue.len()`
   - **Solução**: Ajustar número de workers ou prioridade

3. **Thumbnails corrompidas**
   - **Causa**: Arquivo corrompido ou protegido
   - **Debug**: Verificar `failed_icons` set
   - **Solução**: Adicionar retry ou fallback para ícone padrão

### Como Debugar
```rust
// Verificar fila de thumbnails
println!("Thumbnail queue size: {}", app.thumbnail_queue.len());
println!("Pending thumbnails: {}", app.pending_thumbnails.len());

// Verificar cache hit rate
println!("Cache hits: {}", app.cache_manager.hit_count());
```

---

## 5. Menu de Contexto

### Sequência de Chamadas
```
Right Click
    ↓
ui::context_menu::show_context_menu()
    ↓
infrastructure::windows::native_menu::create_context_menu()
    ↓
Windows Shell API (IContextMenu)
    ↓
User Selection
    ↓
app::operations::context_menu::handle_context_menu_action()
    ↓
Action Execution
```

### Arquivos Envolvidos
- **`src/ui/context_menu.rs`** - UI do menu de contexto
- **`src/infrastructure/windows/native_menu.rs`** - Menu nativo Windows
- **`src/app/operations/context_menu.rs`** - Handler de ações

### Pontos de Bug Comuns
1. **Menu não aparece**
   - **Causa**: Falha ao criar IContextMenu
   - **Debug**: Verificar HRESULT do Windows API
   - **Solução**: Verificar COM está inicializado

2. **Ações não funcionam**
   - **Causa**: Falha na execução do comando
   - **Debug**: Verificar `handle_context_menu_action()`
   - **Solução**: Verificar permissões e existência de comandos

### Como Debugar
```rust
// Adicionar logs no menu creation
eprintln!("[MENU] Creating context menu for: {:?}", path);
eprintln!("[MENU] HRESULT: {:x}", hr.0);
```

---

## 6. Monitoramento de Mudanças (Watcher)

### Sequência de Chamadas
```
Folder Navigation
    ↓
app::operations::watcher::watch_current_folder()
    ↓
[USN] workers::usn_watcher::spawn_usn_watcher()
[Notify] notify::RecommendedWatcher::new()
    ↓
File System Change
    ↓
Event Received
    ↓
app::operations::message_handler::handle_fs_event()
    ↓
app::operations::folder_loading::reload_current_folder()
    ↓
UI Refresh
```

### Arquivos Envolvidos
- **`src/app/operations/watcher.rs`** - Configuração de watchers
- **`src/workers/usn_watcher.rs`** - Watcher USN do NTFS
- **`src/infrastructure/watcher.rs`** - Watcher genérico
- **`src/app/operations/message_handler.rs`** - Handler de eventos

### Pontos de Bug Comuns
1. **Watcher não detecta mudanças**
   - **Causa**: USN não disponível (FAT32)
   - **Debug**: Verificar `usn_watcher_state`
   - **Solução**: Fallback para notify-watcher

2. **Múltiplos reloads**
   - **Causa**: Eventos em cascata do Windows
   - **Debug**: Verificar debounce logic
   - **Solução**: Implementar debounce adequado

### Como Debugar
```rust
// Verificar estado do watcher
println!("Watcher active: {}", app.watcher.is_some());
println!("USN watcher state: {:?}", app.usn_watcher_state);

// Adicionar logs de eventos
eprintln!("[WATCHER] Event: {:?}", event);
```

---

## 7. Sistema de Abas

### Sequência de Chamadas
```
Ctrl+T / New Tab Button
    ↓
tabs::TabManager::create_tab()
    ↓
app::operations::tabs::initialize_tab_state()
    ↓
Navigation History per Tab
    ↓
ui::tab_bar::render_tab_bar()
    ↓
Tab Switch
    ↓
app::operations::tabs::switch_to_tab()
    ↓
Load Tab State
```

### Arquivos Envolvidos
- **`src/tabs/mod.rs`** - Gerenciamento de abas
- **`src/app/operations/tabs.rs`** - Operações de abas
- **`src/ui/tab_bar.rs`** - Renderização da barra de abas

### Pontos de Bug Comuns
1. **Aba não carrega estado**
   - **Causa**: Histórico de navegação corrompido
   - **Debug**: Verificar `tab_manager.active()`
   - **Solução**: Resetar histórico da aba

2. **Memory leak com muitas abas**
   - **Causa**: Recursos não liberados
   - **Debug**: Verificar contadores de referência
   - **Solução**: Implementar cleanup adequado

### Como Debugar
```rust
// Verificar estado das abas
println!("Active tab: {}", app.tab_manager.active().id);
println!("Total tabs: {}", app.tab_manager.tabs.len());

// Verificar histórico
println!("History size: {}", tab.history.len());
```

---

## Checklist de Debug Geral

### Primeiros Passos
1. **Verificar logs**: Executar com redirecionamento de stderr
2. **Verificar estado**: Imprimir campos críticos do `ImageViewerApp`
3. **Verificar workers**: Confirmar threads estão rodando
4. **Verificar cache**: Verificar hits/misses do cache

### Comandos Úteis
```powershell
# Executar com logs completos
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "debug.log"

# Filtrar por categoria
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "ERROR|WARN"

# Verificar performance
Get-Content debug.log | Select-String "frame_time"
```

### Pontos de Verificação
- [ ] libmpv-2.dll está presente?
- [ ] WebView2 Runtime instalado?
- [ ] Permissões de pasta adequadas?
- [ ] Cache SQLite não corrompido?
- [ ] Workers threads rodando?
- [ ] Memória/CPU dentro do esperado?