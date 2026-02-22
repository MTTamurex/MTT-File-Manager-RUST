# Fluxos Principais - MTT File Manager

## Objetivo do Documento
Este documento descreve os fluxos principais do aplicativo, incluindo sequência de chamadas, arquivos envolvidos, pontos de bug comuns e como debugar.

> Nota de estrutura: alguns módulos listados abaixo foram modularizados em diretórios com `mod.rs` (ex.: `folder_loading/`, `message_handler/`, `grid_view/`, `mpv_preview/`).

## 1. Navegação para Pasta

### Sequência de Chamadas
```
User Input (Click/Enter/Double-click)
    ↓
src/ui/app/input.rs - handle_input() / handle_double_click()
    ↓
src/app/operations/navigation/mod.rs - navigate_to_path()
    ↓
src/app/operations/folder_loading/mod.rs - load_folder_contents()
    ↓
src/infrastructure/windows/hdd_directory_reader.rs - read_directory()
    ↓
[Async] src/workers/folder_scanner.rs - scan_folder()
    ↓
src/app/operations/thumbnails.rs - request_thumbnails()
    ↓
src/workers/thumbnail/mod.rs - spawn_thumbnail_workers()
    ↓
src/workers/thumbnail/extraction/ (estágios de extração)
    ↓
UI Update via channels (image_receiver)
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura input do usuário
- **`src/app/operations/navigation/mod.rs`** - Lógica de navegação
- **`src/app/operations/navigation/keyboard.rs`** - Navegação por teclado
- **`src/app/operations/folder_loading/mod.rs`** - Carregamento de pasta
- **`src/infrastructure/windows/hdd_directory_reader.rs`** - Leitura do disco
- **`src/workers/folder_scanner.rs`** - Scanner em background
- **`src/app/operations/thumbnails.rs`** - Solicitação de thumbnails
- **`src/workers/thumbnail/`** - Sistema de thumbnails multi-estágio
- **`src/ui/views/computer_view.rs`** - View especial para "Este Computador"
- **`src/ui/views/grid_view/mod.rs`** - View em grade
- **`src/ui/views/list_view/`** - View em lista

### Pontos de Bug Comuns
1. **Pasta não carrega**
   - **Causa**: Permissões insuficientes, pasta protegida pelo sistema
   - **Debug**: Verificar `AppError::WindowsApi` nos logs
   - **Solução**: Executar como administrador ou verificar ACLs

2. **Performance lenta ao abrir pasta**
   - **Causa**: Pasta com muitos arquivos (>10k) ou arquivos na rede
   - **Debug**: Verificar `is_loading_folder` e `pending_thumbnails.len()`
   - **Solução**: Ajustar `upload_budget_ms`, usar virtualização (já implementada)

3. **Thumbnails não aparecem**
   - **Causa**: Codec não suportado, arquivo corrompido, fila cheia
   - **Debug**: Verificar `failed_thumbnails` no estado, logs de thumbnail
   - **Solução**: Verificar `codec_registry::init_codec_cache()`, limpar cache

### Como Debugar
```rust
// Adicionar logs nos pontos críticos
eprintln!("[NAV] Navigating to: {}", path);
eprintln!("[LOAD] Found {} items", items.len());
eprintln!("[THUMB] Requesting thumbnail for: {:?}", path);

// Verificar estado via debugger
println!("is_loading_folder: {}", app.is_loading_folder);
println!("pending_thumbnails: {}", app.pending_thumbnails.len());
println!("current_generation: {:?}", app.current_generation);
```

---

## 2. Preview de Arquivo

### Sequência de Chamadas
```
User Selection (Click/Navigate)
    ↓
src/ui/views/grid_view/mod.rs - render_item() / handle_click()
    ↓
src/app/operations/selection.rs - handle_selection()
    ↓
src/app/operations/metadata.rs - request_metadata()
    ↓
[Async] src/infrastructure/windows/metadata/mod.rs - extract_media_metadata()
    ↓
src/ui/preview_panel/mod.rs - render_preview_panel()
    ↓
  ├── Imagem: src/ui/preview_panel/image_preview.rs
  ├── Vídeo: src/ui/preview_panel/video_preview/ 
  │         └── src/ui/components/mpv_preview/mod.rs
  ├── GIF: src/ui/components/gif_manager.rs
  └── PDF: src/pdf_viewer/mod.rs - show_pdf_window()
```

### Arquivos Envolvidos
- **`src/ui/views/grid_view/mod.rs`** - Renderização do item
- **`src/app/operations/selection.rs`** - Handler de seleção
- **`src/app/operations/metadata.rs`** - Solicitação de metadados
- **`src/infrastructure/windows/metadata/mod.rs`** - Extração de metadados
- **`src/infrastructure/windows/metadata/image.rs`** - Metadados de imagem
- **`src/infrastructure/windows/metadata/video.rs`** - Metadados de vídeo
- **`src/ui/preview_panel/mod.rs`** - Painel de preview
- **`src/ui/preview_panel/image_preview.rs`** - Preview de imagens
- **`src/ui/preview_panel/video_preview/`** - Preview de vídeo
- **`src/ui/components/media_preview.rs`** - Preview genérico
- **`src/ui/components/mpv_preview/mod.rs`** - Preview de vídeo com mpv
- **`src/ui/components/gif_manager.rs`** - Preview de GIFs
- **`src/pdf_viewer/`** - Preview de PDF

### Pontos de Bug Comuns
1. **Preview não aparece**
   - **Causa**: Arquivo corrompido, formato não suportado, metadados não carregados
   - **Debug**: Verificar `selected_metadata` no estado
   - **Solução**: Verificar logs de `extract_media_metadata()`

2. **Vídeo não reproduz**
   - **Causa**: libmpv-2.dll não encontrada, formato não suportado
   - **Debug**: Verificar console para mensagens de erro mpv
   - **Solução**: Copiar libmpv-2.dll para diretório do executável

3. **PDF não abre**
   - **Causa**: WebView2 Runtime não instalado
   - **Debug**: Verificar `pdf_viewer::warmup()` logs
   - **Solução**: Instalar Microsoft Edge WebView2 Runtime

4. **GIF animado não reproduz**
   - **Causa**: GifManager não inicializado, erro de decodificação
   - **Debug**: Verificar `gif_manager` no estado
   - **Solução**: Reiniciar aplicação, verificar arquivo GIF

### Como Debugar
```powershell
# Verificar logs de metadata
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "metadata"

# Verificar se mpv está funcionando
# Testar arquivo diretamente com mpv
mpv.exe arquivo.mp4

# Verificar logs de PDF
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "PDF|WebView"
```

---

## 3. Operação de Arquivo (Copiar/Mover/Deletar)

### Sequência de Chamadas
```
User Action (Ctrl+C / Ctrl+X / Delete / Context Menu)
    ↓
src/ui/app/input.rs - handle_input()
    ↓
src/app/operations/clipboard_ops.rs - handle_copy() / handle_cut()
    ↓
src/application/clipboard.rs - ClipboardManager
    ↓
User Action (Ctrl+V / Context Menu → Colar)
    ↓
src/app/operations/clipboard_ops.rs - handle_paste()
    ↓
src/workers/file_operation_worker.rs - process_operation()
    ↓
src/infrastructure/windows/shell_operations.rs - copy_items_via_shell() / move_items_via_shell() / delete_items_via_shell()
    ↓
Windows Shell API (IFileOperation)
    ↓
src/application/notification.rs - show_notification()
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura atalhos (Ctrl+C, Ctrl+V, Delete)
- **`src/app/operations/clipboard_ops.rs`** - Operações de clipboard
- **`src/application/clipboard.rs`** - ClipboardManager
- **`src/workers/file_operation_worker.rs`** - Worker de operações
- **`src/infrastructure/windows/shell_operations.rs`** - Shell API (IFileOperation)
- **`src/application/notification.rs`** - Notificações

### Pontos de Bug Comuns
1. **Copiar não funciona**
   - **Causa**: Clipboard bloqueado por outro aplicativo
   - **Debug**: Verificar `ClipboardManager` estado
   - **Solução**: Tentar novamente, fechar aplicativos de clipboard

2. **Colar falha**
   - **Causa**: Destino sem permissão de escrita, arquivo em uso
   - **Debug**: Verificar `FileOperationResult` erro nos logs
   - **Solução**: Executar como administrador ou verificar permissões

3. **Operação lenta**
   - **Causa**: Muitos arquivos grandes, operação na rede
   - **Debug**: Verificar progresso em `file_operation_worker`
   - **Solução**: Operação é assíncrona, aguardar conclusão (notificação)

4. **Deletar não vai para lixeira**
   - **Causa**: Arquivo muito grande, drive não suporta lixeira
   - **Debug**: Verificar logs de `delete_items_via_shell`
   - **Solução**: Shift+Delete para exclusão permanente

### Como Debugar
```rust
// Adicionar logs no worker
eprintln!("[FILE_OP] Starting operation: {:?}", operation);
eprintln!("[FILE_OP] Progress: {}/{}", completed, total);

// Verificar clipboard contents
println!("Clipboard has {} items", app.clipboard.items.len());
println!("Clipboard operation: {:?}", app.clipboard.operation);
```

---

## 4. Geração de Thumbnail

### Sequência de Chamadas
```
Folder Load Complete
    ↓
src/app/operations/thumbnails.rs - request_thumbnails()
    ↓
src/workers/thumbnail/mod.rs - PriorityThumbnailQueue
    ↓
Workers pool (múltiplas threads)
    ↓
src/workers/thumbnail_loader.rs - load_thumbnail()
    ↓
Stage 1: src/workers/thumbnail/extraction/stage1_image_crate.rs (PNG, JPG, GIF, WebP)
    ↓ (fallback)
Stage 2: src/workers/thumbnail/extraction/stage2_wic.rs (WIC)
    ↓ (fallback)
Stage 3: src/workers/thumbnail/extraction/stage3_shell_api.rs (Shell API)
    ↓ (fallback)
Stage 4: src/workers/thumbnail/extraction/stage4_force_extract.rs (Extração forçada)
    ↓ (fallback)
Stage 5: src/workers/thumbnail/extraction/stage5_media_foundation.rs (Media Foundation)
    ↓
Cache em disco (disk_cache.rs)
    ↓
UI via image_receiver channel
    ↓
src/ui/cache.rs - CacheManager::upload_to_gpu()
```

### Arquivos Envolvidos
- **`src/app/operations/thumbnails.rs`** - Coordenação de thumbnails
- **`src/workers/thumbnail/mod.rs`** - Sistema de fila prioritária
- **`src/workers/thumbnail_loader.rs`** - Loader principal
- **`src/workers/thumbnail/extraction/`** - Estágios de extração
  - `stage1_image_crate.rs` - Image crate (formatos comuns)
  - `stage2_wic.rs` - Windows Imaging Component
  - `stage3_shell_api.rs` - IShellItemImageFactory
  - `stage4_force_extract.rs` - Extração forçada
  - `stage5_media_foundation.rs` - Vídeos via MF
- **`src/infrastructure/disk_cache.rs`** - Cache em disco
- **`src/ui/cache.rs`** - Upload GPU

### Pontos de Bug Comuns
1. **Thumbnails não geram para formato específico**
   - **Causa**: Codec não instalado, arquivo corrompido
   - **Debug**: Verificar qual stage está falando nos logs
   - **Solução**: Verificar `codec_registry`, instalar codec Windows

2. **Thumbnails de vídeo não aparecem**
   - **Causa**: Media Foundation não disponível, vídeo codificado em codec exótico
   - **Debug**: Verificar logs de stage5
   - **Solução**: Verificar se vídeo abre no Movies & TV do Windows

3. **Performance de thumbnails lenta**
   - **Causa**: Muitos arquivos grandes, threads bloqueadas
   - **Debug**: Verificar tamanho da fila, threads ativas
   - **Solução**: Ajustar número de workers, limpar cache

### Como Debugar
```rust
// Logs em cada stage
eprintln!("[THUMB] Stage {} trying: {:?}", stage, path);
eprintln!("[THUMB] Success at stage {}", stage);
eprintln!("[THUMB] All stages failed for: {:?}", path);

// Verificar fila
println!("Queue size: {}", app.thumbnail_queue.len());
println!("Pending: {}", app.pending_thumbnails.len());
```

---

## 5. Menu de Contexto

### Sequência de Chamadas
```
User Right-Click
    ↓
src/ui/context_menu.rs - show_context_menu()
    ↓
src/app/operations/context_menu.rs - prepare_context_menu()
    ↓
src/infrastructure/windows/native_menu.rs - create_native_context_menu() [opcional]
    ↓
src/application/context_menu.rs - lógica de contexto
    ↓
User seleciona opção
    ↓
[Nativo] Windows Shell executa ação
    ↓
[Custom] src/app/operations/context_menu.rs - handle_selection()
    ↓
Ação correspondente (copiar, deletar, propriedades, etc)
```

### Arquivos Envolvidos
- **`src/ui/context_menu.rs`** - Renderização UI do menu
- **`src/app/operations/context_menu.rs`** - Handler de contexto
- **`src/application/context_menu.rs`** - Lógica de negócio
- **`src/infrastructure/windows/native_menu.rs`** - Menu nativo do Windows

### Pontos de Bug Comuns
1. **Menu nativo não aparece**
   - **Causa**: COM não inicializado, erro ao criar IContextMenu
   - **Debug**: Verificar logs de `native_menu.rs`
   - **Solução**: Fallback para menu customizado

2. **Opção do menu não funciona**
   - **Causa**: Handler não implementado, comando desconhecido
   - **Debug**: Verificar `handle_selection` logs
   - **Solução**: Implementar handler faltante

---

## 6. Lixeira (Recycle Bin)

### Sequência de Chamadas
```
User navega para Lixeira (sidebar ou digitar path)
    ↓
src/ui/views/computer_view.rs - detecta is_recycle_bin_view = true
    ↓
src/infrastructure/windows/recycle_bin.rs - enumerate_recycle_bin()
    ↓
Cria FileEntrys especiais com deletion_date e recycle_original_path
    ↓
Renderização especial em computer_view.rs
    ↓
User seleciona "Restaurar" no context menu
    ↓
src/app/operations/recycle_bin_ops.rs - restore_items()
    ↓
src/infrastructure/windows/recycle_bin.rs - restore_from_recycle_bin()
```

### Arquivos Envolvidos
- **`src/ui/views/computer_view.rs`** - View da lixeira
- **`src/infrastructure/windows/recycle_bin.rs`** - Operações da lixeira
- **`src/app/operations/recycle_bin_ops.rs`** - Handler de operações
- **`src/domain/file_entry.rs`** - Campos especiais (deletion_date, recycle_original_path)

---

## 7. Navegação por Teclado

### Sequência de Chamadas
```
Key Press (Arrow, Enter, Backspace, etc)
    ↓
src/ui/app/input.rs - handle_key_press()
    ↓
src/app/operations/navigation/keyboard.rs - process_keyboard_navigation()
    ↓
  ├── Setas: Atualiza selected_item, scroll_to_selected = true
  ├── Enter: navigate_to_path() se pasta, preview se arquivo
  ├── Backspace: navigate_to_parent()
  ├── Delete: move_to_recycle_bin() ou delete_permanente()
  ├── F2: Inicia renomeação (renaming_state)
  └── Letras: Busca rápida (jump to file)
    ↓
Atualização de UI
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura de teclas
- **`src/app/operations/navigation/keyboard.rs`** - Lógica de navegação
- **`src/app/operations/selection.rs`** - Seleção

---

## Debugging por Fluxo

### Adicionar Logs Temporários
```rust
// No início de cada função crítica
eprintln!("[FLOW] {} starting", function_name);

// Antes de retornar
eprintln!("[FLOW] {} completed with {:?}", function_name, result);

// Em pontos de decisão
eprintln!("[FLOW] Branch A taken: condition={}", condition);
```

### Inspeção de Estado
```rust
// No final do update() loop
eprintln!("[STATE] items={}, selected={:?}, loading={}", 
    app.items.len(), app.selected_item, app.is_loading_folder);

// Verificar workers
eprintln!("[WORKERS] thumbnails_pending={}, icons_loading={}",
    app.pending_thumbnails.len(), app.loading_icons.len());
```

### PowerShell para Debug
```powershell
# Filtrar logs específicos
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "FLOW|STATE|ERROR" | 
    Tee-Object "debug.log"

# Com timestamp
.\target\release\mtt-file-manager.exe 2>&1 |
    ForEach-Object { "[{0}] {1}" -f (Get-Date -Format "HH:mm:ss"), $_ }
```

---

## 8. Acesso Rápido (Pastas Fixadas na Sidebar)

### Fixar uma Pasta

```
Clique Direito em uma pasta na área principal
    ↓
src/ui/app/input.rs - abre context menu
    ↓
src/app/operations/context_menu.rs - populate_context_menu()
    ↓ (pasta não fixada → ID -60 "Fixar no Acesso Rápido")
    ↓ (pasta já fixada → ID -61 "Remover do Acesso Rápido")
User seleciona opção
    ↓
src/ui/app/menu_handler.rs - handle_context_menu()
    ↓
ID -60 → app.pin_folder(path)
ID -61 → app.unpin_folder(path)
    ↓
src/app/operations/pinned_folder_ops.rs - pin_folder() / unpin_folder()
    ↓
src/infrastructure/disk_cache/pinned_folders.rs - save_pinned_folder() / remove_pinned_folder()
    ↓
app.pinned_folders atualizado → sidebar re-renderiza
```

### Fixar via Drag-and-Drop para a Sidebar

```
User arrasta uma pasta da área principal
    ↓
is_item_dragging = true, drag_payload_paths = [pasta]
    ↓
src/ui/app/panels.rs - render_sidebar_panel()
    ↓ is_folder_dragging = true → SidebarContext.is_folder_dragging = true
src/ui/sidebar.rs - render_sidebar()
    ↓ Seção "Acesso Rápido" renderiza com destaque (borda azul)
User solta o mouse sobre a seção "Acesso Rápido"
    ↓
SidebarAction::PinFolder(path) emitido
    ↓
panels.rs → app.pin_folder(&path)
    ↓
pinned_folder_ops.rs + disk_cache/pinned_folders.rs
```

### Desafixar via Ícone 📌

```
User vê ícone 📌 ao lado do item fixado na sidebar
    ↓ (ícone sempre visível, fica vermelho ao hover)
User clica no ícone 📌
    ↓
src/ui/sidebar.rs - render_pinned_folders()
    ↓ hit-test manual: inp.pointer.interact_pos() dentro de pin_rect
    ↓ action = SidebarAction::UnpinFolder(path)
panels.rs → app.unpin_folder(&path)
    ↓
pinned_folder_ops.rs + disk_cache/pinned_folders.rs - remove_pinned_folder()
    ↓
app.pinned_folders atualizado → item some da sidebar
```

### Reordenar via Drag-and-Drop Interno

```
User arrasta item fixado dentro da seção "Acesso Rápido"
    ↓
src/ui/sidebar.rs - render_pinned_folders()
    ↓ Sense::click_and_drag() em cada item
    ↓ ui.ctx().data_mut() armazena drag_idx por frame
    ↓ Linha indicadora de posição renderizada entre itens
User solta sobre posição destino
    ↓
SidebarAction::ReorderPinnedFolder { from, to } emitido
    ↓
panels.rs → app.reorder_pinned_folder(from, to)
    ↓
pinned_folder_ops.rs - reorder_pinned_folder()
    ↓
disk_cache/pinned_folders.rs - update_pinned_positions()
    ↓
app.pinned_folders reordenado → sidebar atualiza
```

### Arquivos Envolvidos
- **`src/ui/sidebar.rs`** - Renderização, ícone 📌, drag-and-drop
- **`src/ui/app/panels.rs`** - Contexto da sidebar, tratamento de ações
- **`src/app/operations/pinned_folder_ops.rs`** - Lógica de pin/unpin/reorder
- **`src/app/operations/context_menu.rs`** - Itens de menu -60 / -61
- **`src/ui/app/menu_handler.rs`** - Handler dos IDs -60 / -61
- **`src/infrastructure/disk_cache/pinned_folders.rs`** - Persistência SQLite
- **`src/domain/pinned_folder.rs`** - Struct `PinnedFolder`

### Pontos de Bug Comuns
1. **Clique no 📌 navega para a pasta em vez de desafixar**
   - **Causa**: `ui.interact()` em sub-rect conflita com o rect pai
   - **Solução**: Usar `inp.pointer.interact_pos()` + hit-test manual em `pin_rect` (já implementado)

2. **Pastas fixadas não persistem após reiniciar**
   - **Causa**: `save_pinned_folder` não foi chamado ou houve erro no SQLite
   - **Debug**: Verificar logs `[PINNED]` em `pinned_folder_ops.rs`

3. **Sidebar não exibe todos os itens após muitos pins**
   - **Solução**: Sidebar usa `ScrollArea::vertical()` — scroll automático (já implementado)

---

## 9. Drive Watcher (File System Events)

### Sequência de Chamadas
```
User navigates to folder (e.g., "C:\Users\Name")
    ↓
src/app/operations/watcher.rs - watch_current_folder()
    ↓
src/infrastructure/drive_watcher_integration.rs - watch_path()
    ↓
src/infrastructure/drive_watcher.rs - DriveWatcher::new()
    ↓
[Thread] watcher_thread_main() - ReadDirectoryChangesW(handle, "C:\")
    ↓
File system change detected (CREATE/DELETE/MODIFY)
    ↓
Parse FILE_NOTIFY_INFORMATION buffer
    ↓
Send events via channel to UI thread
    ↓
src/app/operations/message_handler/mod.rs - poll_events()
    ↓
Process events: Smart DELETE / CREATE handling
    ↓
Update UI without full reload
```

### Arquivos Envolvidos
- **`src/infrastructure/drive_watcher.rs`** - Core implementation with ReadDirectoryChangesW
- **`src/infrastructure/drive_watcher_integration.rs`** - Manager for multiple drives
- **`src/app/operations/watcher.rs`** - Setup and lifecycle management
- **`src/app/operations/message_handler/mod.rs`** - Event processing and UI updates

### Smart DELETE Handling
```
Drive Watcher detects DELETE event
    ↓
Check if path is in current folder
    ↓
Remove from all_items (source of truth)
    ↓
Filter items Arc to exclude deleted file
    ↓
Update UI immediately (no reload!)
    ↓
Set skip_next_auto_reload = true
```

### Pontos de Bug Comuns
1. **Events not detected**
   - **Causa**: Path mismatch in prefix filtering
   - **Debug**: Check `[DRIVE-WATCHER] Prefix MATCH` logs
   - **Solução**: Verify drive root extraction logic

2. **Double reload on delete**
   - **Causa**: Both drive watcher and notify-watcher active
   - **Debug**: Check `[WATCHER]` logs for watcher selection
   - **Solução**: skip_next_auto_reload flag handles this

3. **UNC paths not working**
   - **Causa**: Drive watcher only works on local drives (C:\, D:\)
   - **Debug**: Check `[WATCHER] UNC/Network path detected`
   - **Solução**: Fallback to notify-watcher for UNC

### Como Debugar
```rust
// Verificar se drive watcher está ativo
eprintln!("[DRIVE-WATCHER] Active: {:?}", self.drive_watcher.is_active());

// Verificar eventos recebidos
for event in self.drive_watcher.poll_events() {
    eprintln!("[DRIVE-WATCHER] Event: {:?}", event);
}

// Verificar smart delete
eprintln!("[FS-WATCH] DELETE: {:?}", path.file_name());
eprintln!("[FS-WATCH] SMART DELETE: Removed from UI without reload");
```

---

## 10. Busca Global (Global Search)

### Sequência de Chamadas
```
User pressiona Ctrl+Shift+F
    ↓
src/ui/app/input.rs - toggle global_search_active
    ↓
src/ui/global_search_overlay.rs - render overlay modal
    ↓
User digita query
    ↓
src/workers/global_search_worker.rs - GlobalSearchRequest::Search
    ↓ (coalescing de queries rápidas)
src/infrastructure/global_search.rs - search(query, offset, limit)
    ↓
open_pipe() → Named Pipe \\.\pipe\MTTFileManagerSearch
    ↓
write_message(SearchRequest::Query) → [4-byte LE length + bincode payload]
    ↓
mtt-search-service (processo separado):
    crates/mtt-search-service/src/ipc_server.rs - handle_client()
        ↓
    crates/mtt-search-service/src/file_index.rs - search_page()
        ↓ (busca substring case-insensitive no nome, com deadline de 5s)
    crates/mtt-search-service/src/path_resolver.rs - resolve_path()
        ↓ (chain de parent FRNs até root)
    SearchResponse::Results → Named Pipe
    ↓
src/infrastructure/global_search.rs - read_response()
    ↓
GlobalSearchResponse::Results → channel → UI
    ↓
src/ui/global_search_overlay.rs - renderiza lista de resultados
    ↓
User seleciona resultado (Enter)
    ↓
src/app/operations/navigation/mod.rs - navigate_to_path()
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura Ctrl+Shift+F
- **`src/ui/global_search_overlay.rs`** - Overlay modal de busca
- **`src/workers/global_search_worker.rs`** - Worker thread (coalescing, retry, status checks)
- **`src/infrastructure/global_search.rs`** - Cliente IPC (Named Pipe)
- **`crates/mtt-search-protocol/src/lib.rs`** - Tipos e serialização bincode
- **`crates/mtt-search-service/src/ipc_server.rs`** - Servidor Named Pipe
- **`crates/mtt-search-service/src/file_index.rs`** - Busca no índice in-memory
- **`crates/mtt-search-service/src/path_resolver.rs`** - Reconstrução de paths
- **`crates/mtt-search-service/src/usn_journal.rs`** - Descoberta/classificação de volumes + USN
- **`crates/mtt-search-service/src/fs_walker.rs`** - Full scan para volumes sem USN

### Fluxo de Startup do Serviço
```
mtt-search-service.exe (Windows Service / console)
    ↓
main.rs - run_indexer()
    ↓
usn_journal::discover_volumes() - GetVolumeInformationW (A-Z)
    ↓
Para cada volume detectado (thread separada):
    ↓
Se usn_supported (NTFS/ReFS):
    index_db.rs - load_volume_state() / load_into_index()
    Se cache válido (journal_id bate):
        usn_journal::read_usn_changes() - catch-up incremental
    Se cache inválido/ausente:
        usn_journal::enumerate_all_files() - FSCTL_ENUM_USN_DATA (full MFT scan)
    file_index::VolumeIndex.state = Ready
    Loop incremental (a cada 2s):
        usn_journal::read_usn_buffer() - sem lock (I/O pura)
        indices.try_write() - aplica mudanças (lock breve, skip se busy)
    Persist SQLite a cada 5 minutos

Se !usn_supported (exFAT/FAT32/FUSE/CryptoFS etc.):
    index_db.rs - load_into_index() (snapshot para startup rápido)
    fs_walker::scan_volume() - full-tree scan iterativo (BFS)
    file_index::VolumeIndex.state = Ready
    save_volume() após cada full scan
    Aguarda novo ciclo:
        30s para fuse/cryptofs/dokan/winfsp
        120s para demais filesystems sem USN

Discovery loop em paralelo: revarre volumes a cada 20s
```

### Pontos de Bug Comuns
1. **"Serviço offline"**
   - **Causa**: Serviço não instalado/iniciado, ou `ERROR_FILE_NOT_FOUND` no pipe
   - **Debug**: `sc.exe query MTTFileManagerSearch`, verificar logs do serviço
   - **Solução**: Instalar e iniciar o serviço

2. **Busca retorna 0 resultados**
   - **Causa**: Índice ainda em estado `Scanning`, ou query não corresponde ao nome dos arquivos indexados
   - **Debug**: `GetStatus` retorna `state: "scanning"` nos volumes
   - **Solução**: Aguardar indexação completar (primeira vez pode ser maior em volumes sem USN)

3. **Delay na primeira busca após idle longo**
   - **Causa**: Páginas de memória do índice foram paged out pelo SO
   - **Debug**: Verificar se `WarmIndex` foi chamado no startup do worker
   - **Solução**: O worker chama `warm_index()` automaticamente no startup

4. **Icon flickering nos resultados de busca**
   - **Causa**: LRU cache de ícones muito pequeno para a quantidade de resultados
   - **Debug**: Verificar tamanho do `icon_cache` no `icon_loader.rs`
   - **Solução**: Cache LRU de 512 entradas (default atual)

5. **Resultados desatualizados em exFAT/FAT32/CryptoFS**
   - **Causa**: Volumes sem USN atualizam por re-scan periódico (não há loop incremental de 2s)
   - **Debug**: Verificar logs `[SCAN]` no serviço e o filesystem detectado
   - **Solução**: Aguardar próximo ciclo (30s/120s) ou reiniciar o serviço para forçar novo scan

### Como Debugar
```powershell
# Verificar status do serviço
sc.exe query MTTFileManagerSearch

# Rodar serviço em modo console com logs
.\target\release\mtt-search-service.exe run-console

# Filtrar logs de busca no app
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "GLOBAL-SEARCH|IPC"

# Verificar se o pipe existe
[System.IO.Directory]::GetFiles("\\.\pipe\") | Select-String "MTTFileManager"
```

---

## 11. Folder Cover Composition (Custom Folder Preview)

### Visão Geral
O sistema de folder covers customizados substitui completamente a geração de previews via Windows Shell API (`IThumbnailCache` / `IShellItemImageFactory`). A Shell API gerava covers com problemas frequentes: fundos pretos, ícones genéricos em vez de thumbnails, e previews quebrados.

A nova implementação compõe covers programaticamente com 3 camadas PNG usando a crate `image`:
1. **folder_back_512.png** — silhueta de fundo da pasta (layer traseiro)
2. **Thumbnail do conteúdo** — primeira imagem/vídeo encontrada dentro da pasta
3. **folder_front_512.png** — aba frontal da pasta (overlay superior)

### Sequência de Chamadas
```
UI renderiza folder slot (grid/list view)
    ↓
src/ui/components/item_slot/folder_slot.rs - request_folder_preview_load()
    ↓ (todas as pastas normais, sem guard de has_cover)
Channel send → folder_preview_sender
    ↓
src/workers/folder_preview_worker.rs - spawn_folder_preview_worker()
    ↓
  ├── FAST PATH: SQLite disk cache check (~1ms)
  │   → disk_cache.get_folder_preview_cache(&path)
  │   → Verificação de staleness: folder mtime > cache created_at?
  │   → Se cache fresh: retorna imediatamente
  │
  ├── SLOW PATH (com mídia):
  │   ↓
  │   find_folder_preview_item(folder_path) → primeira imagem/vídeo
  │   ↓
  │   generate_thumbnail_hybrid(media_path) → pipeline 5 estágios
  │   ↓
  │   composer.compose(content_rgba, w, h) → back + thumbnail + front
  │   ↓
  │   disk_cache.put_folder_preview_cache() → persiste em SQLite (WebP)
  │
  └── FALLBACK (sem mídia):
      ↓
      composer.compose_empty() → back + front (sem thumbnail)
      ↓
      disk_cache.put_folder_preview_cache() → persiste em SQLite
    ↓
FolderPreviewData via channel → UI
    ↓
src/ui/cache.rs - CacheManager::upload_to_gpu()
```

### Arquivos Envolvidos
- **`src/embedded_assets.rs`** - PNGs embutidos via `include_bytes!` (folder_back_512.png, folder_front_512.png)
- **`src/infrastructure/folder_compose.rs`** - `FolderComposer` (decodifica layers uma vez, compõe covers)
- **`src/workers/folder_preview_worker.rs`** - Worker thread (cache check → compose → envio)
- **`src/app/init_bootstrap.rs`** - Cria `Arc<FolderComposer>` no startup
- **`src/app/init_workers/filesystem_workers.rs`** - Passa `Arc<FolderComposer>` aos workers
- **`src/ui/components/item_slot/folder_slot.rs`** - Solicita preview para todas as pastas
- **`src/ui/preview_panel/fallback_renderer.rs`** - Usa cover customizado no painel de detalhes
- **`src/infrastructure/disk_cache/folder_previews.rs`** - CRUD de previews no SQLite

### Arquitetura do FolderComposer
```
┌─────────────────────────────────────────────────────────────┐
│                  FolderComposer (Arc, shared)                │
│                                                              │
│  Startup (uma vez):                                          │
│    folder_back_512.png  → decode → resize(256px) → back     │
│    folder_front_512.png → decode → resize(256px) → front    │
│                                                              │
│  compose(content_rgba):          compose_empty():            │
│    canvas 256×173 (transparent)    canvas 256×173            │
│    ├── overlay back (bottom-aligned)  ├── overlay back       │
│    ├── thumbnail (fill-width,         └── overlay front      │
│    │   top-aligned, cropped)                                 │
│    └── overlay front (bottom-aligned)                        │
│                                                              │
│  Constantes:                                                 │
│    OUTPUT_W = 256px                                          │
│    CONTENT_MARGIN: L=10, R=10, T=30, B=0                    │
└─────────────────────────────────────────────────────────────┘
```

### Performance
| Operação | Tempo |
|----------|-------|
| Decode dos PNGs (startup, uma vez) | ~2ms |
| Cache hit SQLite (NVMe) | ~1ms |
| Composição com thumbnail | ~1-2ms |
| Composição vazia (sem mídia) | ~0.5ms |
| Shell API anterior (COM interop) | 20-200ms |

### Cache e Invalidação
- Covers compostos são armazenados em SQLite (tabela `folder_previews`) como blobs WebP
- No próximo acesso, o worker verifica `created_at` do cache vs `mtime` da pasta
- Se a pasta foi modificada (arquivo adicionado/removido), o cover é recomposto
- O `CacheManager` (GPU) também armazena texturas em memória para acesso imediato

### Pontos de Bug Comuns
1. **Folder cover não aparece**
   - **Causa**: Worker não recebeu a solicitação, ou `folder_preview_sender` congestionado
   - **Debug**: Verificar logs `[FOLDER PREVIEW]` para DB HIT/MISS/STALE
   - **Solução**: `request_folder_preview_load()` é chamado para toda pasta normal

2. **Cover mostra pasta vazia quando tem mídia**
   - **Causa**: `find_folder_preview_item()` não encontrou arquivos de mídia (extensão não reconhecida) ou pipeline falhou
   - **Debug**: Verificar logs `[FOLDER PREVIEW] Custom compose FAILED`
   - **Solução**: Verificar extensões suportadas em `find_folder_preview_item`

3. **Cover desatualizado após adicionar/remover arquivos**
   - **Causa**: Cache SQLite ainda fresh (mtime não atualizado pelo OS para o diretório)
   - **Debug**: Verificar `is_stale` no worker
   - **Solução**: O DriveWatcher invalida o cache quando detecta mudanças na pasta

4. **Performance lenta na primeira vez em pasta com muitas subpastas**
   - **Causa**: Todas as subpastas precisam composição (sem cache)
   - **Debug**: Verificar logs de composição e fila do worker
   - **Solução**: Workers processam em paralelo (2-6 threads baseado em CPU count)

### Como Debugar
```rust
// Logs automáticos no worker:
// [FOLDER PREVIEW] DB HIT "FolderName" (256x173, 0.8ms)
// [FOLDER PREVIEW] DB MISS "FolderName" (0.1ms)  
// [FOLDER PREVIEW] DB STALE "FolderName" (folder modified after cache)
// [FOLDER PREVIEW] Custom compose SUCCESS "FolderName" via "photo.jpg" (1.5ms)
// [FOLDER PREVIEW] Custom compose FAILED for "FolderName"

// Verificar composer no startup:
// [FOLDER COMPOSE] Layers decoded — back: 256×173, front: 256×112, canvas: 256×173
```

---

*Última atualização: 2026-02-22 (adicionado fluxo de Folder Cover Composition customizada)*

