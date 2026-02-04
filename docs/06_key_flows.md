# Fluxos Principais - MTT File Manager

## Objetivo do Documento
Este documento descreve os fluxos principais do aplicativo, incluindo sequência de chamadas, arquivos envolvidos, pontos de bug comuns e como debugar.

## 1. Navegação para Pasta

### Sequência de Chamadas
```
User Input (Click/Enter/Double-click)
    ↓
src/ui/app/input.rs - handle_input() / handle_double_click()
    ↓
src/app/operations/navigation/mod.rs - navigate_to_path()
    ↓
src/app/operations/folder_loading.rs - load_folder_contents()
    ↓
src/infrastructure/windows/hdd_directory_reader.rs - read_directory()
    ↓
[Async] src/workers/folder_scanner.rs - scan_folder()
    ↓
src/app/operations/thumbnails.rs - request_thumbnails()
    ↓
src/workers/thumbnail/mod.rs - spawn_thumbnail_workers()
    ↓
src/workers/thumbnail/extraction/stage*.rs (estágios de extração)
    ↓
UI Update via channels (image_receiver)
```

### Arquivos Envolvidos
- **`src/ui/app/input.rs`** - Captura input do usuário
- **`src/app/operations/navigation/mod.rs`** - Lógica de navegação
- **`src/app/operations/navigation/keyboard.rs`** - Navegação por teclado
- **`src/app/operations/folder_loading.rs`** - Carregamento de pasta
- **`src/infrastructure/windows/hdd_directory_reader.rs`** - Leitura do disco
- **`src/workers/folder_scanner.rs`** - Scanner em background
- **`src/app/operations/thumbnails.rs`** - Solicitação de thumbnails
- **`src/workers/thumbnail/`** - Sistema de thumbnails multi-estágio
- **`src/ui/views/computer_view.rs`** - View especial para "Este Computador"
- **`src/ui/views/grid_view.rs`** - View em grade
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
src/ui/views/grid_view.rs - render_item() / handle_click()
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
  │         └── src/ui/components/mpv_preview.rs
  ├── GIF: src/ui/components/gif_manager.rs
  └── PDF: src/pdf_viewer/mod.rs - show_pdf_window()
```

### Arquivos Envolvidos
- **`src/ui/views/grid_view.rs`** - Renderização do item
- **`src/app/operations/selection.rs`** - Handler de seleção
- **`src/app/operations/metadata.rs`** - Solicitação de metadados
- **`src/infrastructure/windows/metadata/mod.rs`** - Extração de metadados
- **`src/infrastructure/windows/metadata/image.rs`** - Metadados de imagem
- **`src/infrastructure/windows/metadata/video.rs`** - Metadados de vídeo
- **`src/ui/preview_panel/mod.rs`** - Painel de preview
- **`src/ui/preview_panel/image_preview.rs`** - Preview de imagens
- **`src/ui/preview_panel/video_preview/`** - Preview de vídeo
- **`src/ui/components/media_preview.rs`** - Preview genérico
- **`src/ui/components/mpv_preview.rs`** - Preview de vídeo com mpv
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

## 8. Drive Watcher (File System Events)

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
src/app/operations/message_handler.rs - poll_events()
    ↓
Process events: Smart DELETE / CREATE handling
    ↓
Update UI without full reload
```

### Arquivos Envolvidos
- **`src/infrastructure/drive_watcher.rs`** - Core implementation with ReadDirectoryChangesW
- **`src/infrastructure/drive_watcher_integration.rs`** - Manager for multiple drives
- **`src/app/operations/watcher.rs`** - Setup and lifecycle management
- **`src/app/operations/message_handler.rs`** - Event processing and UI updates

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

*Última atualização: 2026-02-04 (Drive Watcher implementation)*
