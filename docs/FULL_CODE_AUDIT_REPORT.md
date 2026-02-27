# Relatório Completo de Auditoria Técnica — MTT File Manager (Rust/egui)

**Data:** 27 de Fevereiro de 2026 (atualizado com requisitos de UX)  
**Base de código:** 290 arquivos `.rs`, ~49.300 linhas de código  
**Análise baseada em:** Código-fonte real (nenhuma documentação foi utilizada como referência)  
**Escopo:** Todas as camadas — UI rendering, application logic, infrastructure, workers, viewers, domain

---

## Requisitos do Projeto (Restrições de Design)

Os seguintes requisitos são **obrigatórios** e têm prioridade sobre qualquer outra consideração técnica. Todo o relatório e roadmap estão alinhados a estes princípios:

### R-1: Experiência do Usuário (UX) é Prioridade Máxima
> A Interface do Usuário (UI) **não deve travar ou congelar sob nenhuma circunstância**.
> A navegação deve ser **obrigatoriamente fluida e suave**.

**Implicação técnica:** Qualquer operação de I/O, chamada Shell/COM, ou processamento pesado que ocorra na thread de UI é um **bug crítico por definição**, independentemente de sua frequência.

### R-2: Carregamento de Mídia em Velocidade Máxima
> Thumbnails, previews, ícones de arquivos e ícones de pastas devem carregar em **velocidade máxima**.
> Esse carregamento deve ser **assíncrono** (em segundo plano) para garantir que não haja impacto ou travamento na UI.

**Implicação técnica:** Workers de thumbnail/icon/preview devem ter throughput maximizado. Gargalos como `Arc<Mutex<Receiver>>` (serialização de dispatch), `ORDER BY RANDOM()` (GC lento), e reader/writer Mutex stall são violações diretas deste requisito.

### R-3: PROIBIDO O USO DE PLACEHOLDERS (Regra Rígida)
> O sistema deve seguir o padrão de gerenciadores de arquivos renomados do mercado, que **exibem diretamente o item final** e **não utilizam imagens temporárias (placeholders)** na interface.

**Implicação técnica:**
- **PROIBIDO:** Spinners, retângulos cinza, ícones genéricos temporários, emojis, ou qualquer imagem que será substituída depois.
- **PERMITIDO:** O espaço do item pode ficar vazio/sem conteúdo visual enquanto o ícone/thumbnail real carrega assincronamente. Quando o conteúdo final chegar, ele aparece diretamente — sem transição de placeholder → final.
- **Estratégia:** Ícones de extensão (`.jpg`, `.pdf`, `.exe`) devem ser pré-cacheados por tipo de extensão para exibição instantânea — esses SÃO o conteúdo final para arquivos sem thumbnail. Thumbnails reais substituem o ícone de extensão quando prontos.
- **Violações atuais encontradas no código:**
  - `folder_slot.rs` L136-165: **Spinner rotativo** sobre fundo cinza enquanto folder preview carrega
  - `file_slot.rs` L101: **Retângulo cinza** (`rect_filled(..., Color32::from_gray(248))`) como fallback visual
  - `folder_slot.rs` L125, L133: **Retângulo cinza** para pastas sem preview
  - `computer_view.rs` L43: Drive icon fallback emoji `💽`

---

## Índice

1. [Visão Geral do Código](#1-visão-geral-do-código)
2. [Relatório de Bugs Encontrados](#2-relatório-de-bugs-encontrados)
   - [Críticos](#crítico-7-issues)
   - [Altos](#alto-8-issues)
   - [Médios](#médio-20-issues)
   - [Baixos](#baixo-15-issues)
3. [Oportunidades de Otimização e Performance](#3-oportunidades-de-otimização-e-performance)
4. [Melhorias de Estabilidade](#4-melhorias-de-estabilidade)
5. [Padrões Positivos (Preservar)](#5-padrões-positivos-preservar)
6. [Plano de Implementação (Roadmap)](#6-plano-de-implementação-roadmap)

---

## 1. Visão Geral do Código

### Arquitetura

O projeto é um gerenciador de arquivos nativo para Windows construído com `eframe`/`egui`, organizado em camadas claras:

| Camada | Responsabilidade |
|--------|-----------------|
| `app/` | Estado da aplicação (~560 campos no struct principal), inicialização, bootstrap de workers |
| `ui/` | Renderização egui (views, components, panels, toolbar, sidebar, preview panel) |
| `application/` | Lógica de negócio (clipboard, navegação, renomeação, sorting, watcher) |
| `infrastructure/` | I/O nativo Windows (NTFS reader, drive watcher via RDCW, disk cache SQLite, shell operations) |
| `workers/` | Threads de background (thumbnails, file operations, global search, folder preview) |
| `image_viewer/`, `pdf_viewer/`, `video_player/` | Visualizadores standalone (processos separados) |
| `domain/` | Tipos centrais (`FileEntry`, erros, folder locks) |
| `tabs/` | Sistema de abas com snapshot de estado |

### Qualidade Geral

A base de código demonstra **maturidade técnica significativa**. O projeto já implementa padrões avançados:

- Sistema de gerações para invalidação de thumbnails stale
- Fila de prioridade com distinção HDD/SSD
- Throttling adaptativo de GPU uploads
- Memory maintenance com limites soft/hard baseados em working set
- Virtualização de scroll manual com overscan adaptativo
- Proteção contra OneDrive cloud-only files
- Budget system para icon loading (max 6 lookups / 4ms por 16ms window)
- 1-second TTL atomic caching para RAM/VRAM syscalls
- `safe_unwrap!`/`safe_expect!` macros no domain layer
- FxHashSet/FxHashMap para hashing rápido de PathBuf keys
- Debounced preferences saving (evita I/O excessivo)
- Adaptive batch sizing baseado em frame time

O uso de `.unwrap()` é **notavelmente baixo** (31 ocorrências totais em ~49K linhas), concentrado em workers onde o impacto de panic é contido.

### Métricas de `unsafe`

| Arquivo | Count | Justificativa |
|---------|-------|---------------|
| `pdf_viewer/webview.rs` | 61 | COM/WebView2 interop (inevitável) |
| `infrastructure/windows/metadata/utils.rs` | 37 | Win32 API para metadata |
| `infrastructure/windows/metadata/video.rs` | 20 | Media Foundation COM |
| `infrastructure/ntfs_reader.rs` | 10 | NTFS USN journal (direct I/O) |
| _Demais (~25 arquivos)_ | 3-8 cada | Win32 FFI necessário |

A maioria do `unsafe` é justificada por necessidade de FFI com Win32. O ponto de atenção é `pdf_viewer/webview.rs` com 61 ocorrências e `thread_loop.rs` com um bloco `unsafe` de ~140 linhas.

---

## 2. Relatório de Bugs Encontrados

### CRÍTICO (7 issues)

> **Nota de severidade:** Com os requisitos R-1/R-2/R-3 em vigor, qualquer operação bloqueante na UI thread ou uso de placeholders visuais é classificado como **Crítico**, independentemente da frequência.

#### C-1: `is_dir()` bloqueante na UI thread durante context menu

| Campo | Valor |
|---|---|
| **Arquivo** | `src/app/operations/context_menu.rs` L180 |
| **Impacto** | UI freeze de 5–60 segundos em caminhos OneDrive/rede |
| **Viola** | R-1 (UI não pode travar) |

**Descrição:** `std::path::Path::new(target_path).is_dir()` é chamado ao clicar com botão direito. Em caminhos OneDrive cloud-only ou rede, este call bloqueia a thread de UI enquanto o OS tenta resolver/hidratar o arquivo.

**Código atual:**
```rust
// context_menu.rs:180
if std::path::Path::new(target_path).is_dir() {
```

**Fix:** Substituir por `item.is_dir` do `FileEntry` já disponível:
```rust
let is_dir_item = _item_index
    .and_then(|idx| self.items.get(idx))
    .map(|item| item.is_dir)
    .unwrap_or(false);
```

---

#### C-2: `extract_shell_menu()` bloqueante na UI thread

| Campo | Valor |
|---|---|
| **Arquivo** | `src/app/operations/context_menu.rs` L205 |
| **Impacto** | UI freeze de 1–10+ segundos no right-click |
| **Viola** | R-1 (UI não pode travar) |

**Descrição:** Shell extensions de terceiros (antivírus, cloud sync) podem travar esta chamada COM indefinidamente, congelando a UI inteira durante right-click.

**Fix:** Executar `extract_shell_menu` em thread de background. Mostrar apenas os items internos do menu imediatamente; quando os items do Shell estiverem prontos, inserir diretamente no menu (atualizando os items existentes, sem placeholder visual).

---

#### C-3: `show_properties_for_idx` COM bloqueante na UI thread

| Campo | Valor |
|---|---|
| **Arquivo** | `src/app/operations/file_ops.rs` L166-L198 |
| **Impacto** | UI freeze no Alt+Enter / Properties |
| **Viola** | R-1 (UI não pode travar) |

**Descrição:** `extract_shell_menu` + `invoke_menu_command` executados na thread de UI para Alt+Enter/Properties.

**Fix:** Dispatch operações Shell para thread de background, ou usar `ShellExecuteExW` com verbo `"properties"` (mais leve).

---

#### C-4: Spinner placeholder em folder preview (`folder_slot.rs`)

| Campo | Valor |
|---|---|
| **Arquivo** | `src/ui/components/item_slot/folder_slot.rs` L133-165 |
| **Impacto** | Violação direta da regra de proibição de placeholders |
| **Viola** | R-3 (PROIBIDO placeholders) |

**Descrição:** Enquanto o folder preview composto não está pronto, o código exibe: (1) um retângulo cinza (`Color32::from_gray(245)`) preenchendo a área da pasta, e (2) um spinner animado (arco rotativo desenhado com 20 pontos). Ambos são placeholders visuais temporários que serão substituídos pelo preview final.

**Código atual:**
```rust
// folder_slot.rs:143-165
ui.painter().rect_filled(folder_rect, 4.0, egui::Color32::from_gray(245));
// ... spinner drawing ...
ui.painter().add(egui::Shape::line(points, stroke));
```

**Fix:** Remover o spinner e o retângulo cinza. Enquanto o preview da pasta não carregou, mostrar o **ícone de pasta do sistema** (já disponível via `ctx.icon_loader.folder_icon()`, que é o ícone final para pastas sem conteúdo visual). Quando o preview composto chegar, ele substitui o ícone diretamente.
```rust
// Usar ícone de pasta do sistema (conteúdo final, não placeholder)
if let Some(folder_icon) = ctx.icon_loader.folder_icon() {
    paint_texture_centered(ui, folder_icon.id(), folder_icon.size_vec2(), folder_rect);
}
// Não desenhar spinner nem retângulo cinza
```

---

#### C-5: Retângulo cinza como fallback visual em file items (`file_slot.rs`)

| Campo | Valor |
|---|---|
| **Arquivo** | `src/ui/components/item_slot/file_slot.rs` L101-103 |
| **Impacto** | Placeholder visual para arquivos sem thumbnail carregado |
| **Viola** | R-3 (PROIBIDO placeholders) |

**Descrição:** Quando um arquivo não tem thumbnail, um retângulo cinza (`Color32::from_gray(248)`) é desenhado como fundo, com o ícone do Windows centralizado por cima. O retângulo cinza é visualmente um placeholder.

**Código atual:**
```rust
// file_slot.rs:101-103
ui.painter().rect_filled(thumb_rect, 4.0, egui::Color32::from_gray(248));
```

**Fix:** Remover o retângulo cinza. Renderizar apenas o ícone do Windows Shell diretamente (que É o conteúdo final para esse tipo de arquivo). Se o ícone ainda não carregou assincronamente, o espaço deve ficar vazio — sem retângulo colorido temporário.
```rust
if let Some(icon_texture) = file_icon {
    // Ícone do sistema = conteúdo final, renderizar diretamente
    let icon_rect = egui::Rect::from_center_size(thumb_rect.center(), egui::vec2(icon_size, icon_size));
    ui.painter().image(icon_texture.id(), icon_rect, UV, egui::Color32::WHITE);
}
// Se não tem ícone ainda: espaço vazio (sem rect_filled cinza)
```

---

#### C-6: `extract_drive_icon()` bloqueante no render loop (`computer_view.rs`)

| Campo | Valor |
|---|---|
| **Arquivo** | `src/ui/views/computer_view.rs` L43-57 |
| **Impacto** | UI freeze de 50-200ms por drive no primeiro render |
| **Viola** | R-1 (UI não pode travar), R-2 (carregamento assíncrono obrigatório) |

**Descrição:** Quando o ícone de um drive não está no cache LRU, `windows::extract_drive_icon()` é chamado **sincronamente dentro do render loop**. Esta chamada acessa APIs Shell do Windows (`SHGetFileInfo`/`IExtractIcon`) que podem bloquear por 50-200ms, especialmente em drives de rede ou USB.

**Código atual:**
```rust
// computer_view.rs:42-43
if let Ok((rgba_data, width, height)) =
    windows::extract_drive_icon(disk_path, IconSize::Small) // BLOCKING
```

**Fix:** Mover extração de ícones de drive para background thread. O `IconLoader` já tem o padrão async (`icon_result_tx`/`icon_result_rx`) — usar o mesmo canal para drive icons. Pré-extrair ícones de drives durante a detecção de drives (que já ocorre em background), não durante o render.

---

#### C-7: Retângulos cinza como fallback em pastas sem preview em caminhos especiais

| Campo | Valor |
|---|---|
| **Arquivo** | `src/ui/components/item_slot/folder_slot.rs` L125, L133 |
| **Impacto** | Placeholder visual |
| **Viola** | R-3 (PROIBIDO placeholders) |

**Descrição:** Em dois pontos adicionais, `rect_filled(..., Color32::from_gray(245))` é usado como fallback para pastas que não têm ícone ou preview.

**Fix:** Substituir por espaço vazio ou pelo ícone de pasta do sistema. Mesmo tratamento de C-4.

---

### ALTO (8 issues)

#### H-1: Clones per-frame desnecessários em `panels.rs`

| Campo | Valor |
|---|---|
| **Arquivo** | `src/ui/app/panels.rs` L94-97, L412 |
| **Impacto** | ~4 heap allocations/frame × 60 FPS = ~240 allocations/segundo |

**Dados clonados a cada frame:**
- `app.drive_state.disks.clone()` — `Vec<(String, String)>` (L94)
- `app.navigation_state.current_path.clone()` — `String` (L95)
- `app.cache_manager.computer_icon.clone()` — `TextureHandle` (L97)
- `app.selected_file.clone()` — `FileEntry` inteiro com PathBuf, String, metadata (L412)

**Fix:** Passar referências (`&str`, `&[T]`) aos métodos de rendering. Retornar `Option<&FileEntry>` em vez de clonar.

---

#### H-2: `create_shortcut` deixa arquivo placeholder órfão em caso de erro

| Campo | Valor |
|---|---|
| **Arquivo** | `src/application/file_operations.rs` L233-L267 |
| **Impacto** | Arquivos `.lnk` de 0 bytes no disco |

**Descrição:** Cria um arquivo `.lnk` de 0 bytes via `create_new(true)` como placeholder. Se qualquer chamada COM subsequente falhar (6 pontos de `?` return), o arquivo vazio permanece no disco.

**Fix:**
```rust
if let Err(e) = com_result {
    let _ = std::fs::remove_file(&candidate);
    return Err(e);
}
```

---

#### H-3: `file_op_sender.send()` silenciosamente falha se worker morreu

| Campo | Valor |
|---|---|
| **Arquivo** | Múltiplos locais em `file_ops.rs`, `clipboard_ops.rs`, `recycle_bin_ops.rs` |
| **Impacto** | Indicador permanente de "operações em progresso" |

**Descrição:** `let _ = self.file_operation_state.file_op_sender.send(...)` ignora erros. Se o worker crashou, `file_ops_in_progress` é incrementado mas nunca decrementado.

**Fix:** Verificar resultado do send e decrementar o counter on failure (padrão já existe em `drag_drop.rs`).

---

#### H-4: HANDLE leak on panic em `ntfs_reader.rs`

| Campo | Valor |
|---|---|
| **Arquivo** | `src/infrastructure/ntfs_reader.rs` L82-L186 |
| **Impacto** | Handle leak → resource exhaustion sob panics repetidos |

**Descrição:** `HANDLE` de `CreateFileW` (L82) só é fechado em L186. Sem RAII guard — se panic ocorrer entre estas linhas, o handle vaza.

**Fix:** Wrapper RAII:
```rust
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) { unsafe { let _ = CloseHandle(self.0); } }
}
```

---

#### H-5: Transações SQLite manuais sem rollback safety

| Campo | Valor |
|---|---|
| **Arquivo** | `src/infrastructure/disk_cache/gc.rs` L177-L192, L294-L309 |
| **Impacto** | Database corruption após panic; cache de thumbnails non-functional |

**Descrição:** `BEGIN TRANSACTION`/`COMMIT` via SQL raw com erros ignorados (`let _`). Se panic ocorrer entre begin e commit, a conexão fica em estado sujo.

**Fix:** Usar `conn.transaction()?` (RAII — auto-rollback on Drop).

---

#### H-6: Shutdown incompleto de workers

| Campo | Valor |
|---|---|
| **Arquivo** | `src/ui/app/lifecycle.rs` L135 |
| **Impacto** | Operações de arquivo interrompidas mid-operation, COM não finalizado |

**Descrição:** `handle_exit()` apenas chama `thumbnail_queue.shutdown()`. ~12+ workers não recebem sinal de shutdown.

**Fix:** Dropar todos os `Sender` halves em `handle_exit()` para que workers saiam naturalmente via `recv() -> Err`.

---

#### H-7: WebView2 `Controller::Close()` não chamado antes de Release

| Campo | Valor |
|---|---|
| **Arquivo** | `src/pdf_viewer/webview.rs` L132-L141 |
| **Impacto** | Processos `msedgewebview2.exe` órfãos após fechar PDF viewer |

**Descrição:** O `Drop` de `WebViewState` faz Release de COM objects sem chamar `Controller::Close()` primeiro, conforme exigido pela documentação WebView2.

**Fix:** Chamar `Close()` no controller antes de `Release()`.

---

#### H-8: Drag preview carrega ícones com blocking Shell calls todo frame

| Campo | Valor |
|---|---|
| **Arquivo** | `src/app/operations/drag_drop.rs` L161 |
| **Impacto** | UI freeze durante drag — chamadas Shell bloqueantes a cada frame enquanto arrastando |
| **Viola** | R-1 (UI não pode travar) |

**Descrição:** `render_item_drag_preview` chama APIs Shell do Windows para carregar ícones **a cada frame** durante operações de drag-and-drop. Isso causa micro-freezes perceptíveis durante o arrasto de itens.

**Fix:** Cachear a textura do ícone em `begin_item_drag`:
```rust
// Em begin_item_drag(): pre-load e cachear o ícone para drag preview
let drag_icon = extract_icon(path, IconSize::Small).ok().map(|rgba| {
    ctx.load_texture("drag-icon", egui::ColorImage::from_rgba(...), Default::default())
});
// Em render_item_drag_preview(): usar textura cacheada, sem Shell calls
```

---

### MÉDIO (20 issues)

#### Camada UI — Rendering / Per-frame allocations

| ID | Arquivo | Linhas | Problema |
|----|---------|--------|----------|
| M-2 | `src/ui/app/panels.rs` | L161 | `refresh_selected_metadata()` chamado todo frame com preview panel aberto — clona path + probe caches mesmo sem mudança |
| M-3 | `src/ui/toolbar.rs` | L266-328 | Breadcrumb path parsing: decomposição, `PathBuf` accumulations, `to_string_lossy()` + `to_string()` por componente a cada frame |
| M-4 | `src/ui/tab_bar/tabs_renderer.rs` | L155-186 | Tab title truncation: binary search + `format!("{}...", ...)` O(log n) allocations por tab por frame |
| M-5 | `src/ui/context_menu.rs` | L75-85 | Context menu `Vec::collect()` × 3 todo frame enquanto menu aberto (`primary_items`, `secondary_items`, `overflow_items`) |
| M-6 | `src/ui/views/list_view/mod.rs` | L36, L121 | `TRUNCATION_CACHE` usa HashMap com `clear()` total a 2000 entries em vez de LRU — causa cache avalanche periódico |
| M-7 | `src/ui/svg_icons.rs` | L53 | `icon_name.to_string()` em cache key do SVG a cada lookup — allocation desnecessária |
| M-8 | `src/ui/app/notifications.rs` | L31 | `format!("toast_{}", i)` por notification por frame para egui ID |
| M-9 | `src/ui/tab_bar/tabs_renderer.rs` | L193, L226 | `format!("speaker_{}", idx)` e `format!("close_{}", idx)` por tab por frame |
| M-10 | `src/ui/components/item_slot/folder_slot.rs` | L153-159 | ~~Spinner cria `Vec<Pos2>` de 20 elementos por folder loading por frame~~ → **Absorvido por C-4** (spinner será removido) |
| M-11 | `src/ui/views/list_view/mod.rs` | L151 | `char_boundaries: Vec<usize>` allocation por truncation cache miss |
| M-12 | `src/ui/app/panels.rs` | L422-488 | `FileEntry` construído todo frame quando nenhum arquivo selecionado (Path/String allocs) |
| M-13 | `src/ui/status_bar.rs` | L342, L348 | `format!()` para RAM/VRAM labels todo frame (valores mudam 1x/segundo) |

#### Camada Infrastructure / Workers

| ID | Arquivo | Linhas | Problema |
|----|---------|--------|----------|
| M-14 | `src/infrastructure/ntfs_reader.rs` | L167 | Cast `i64 as u64` sem guard para valores negativos (metadata NTFS corrompida) |
| M-15 | `src/infrastructure/disk_cache/gc.rs` | L105 | `ORDER BY RANDOM()` força full table scan O(n log n) no GC incremental |
| M-16 | `src/infrastructure/disk_cache/cleanup.rs` | L15 | 6+ DELETEs sem transação — cada um auto-comita individualmente |
| M-17 | `src/infrastructure/disk_cache.rs` | L225 | Fallback reader=writer causa stall de reads durante GC/writes |
| M-18 | `src/workers/folder_preview_worker.rs` | L53 | `Arc<Mutex<Receiver>>` serializa dispatch — apenas 1 de N threads espera work (**viola R-2: velocidade máxima**) |
| M-19 | `src/workers/folder_preview_worker.rs` | L48 | COM init sem RAII guard (panic-unsafe) |
| M-20 | `src/infrastructure/global_search.rs` | L258 | Busy-wait polling com `PeekNamedPipe` + sleep(15ms) |
| M-21 | `src/infrastructure/drive_watcher/buffer_parser.rs` | L28 | Unsafe buffer parsing confia em kernel data sem bounds check |

#### Camada Application

| ID | Arquivo | Linhas | Problema |
|----|---------|--------|----------|
| M-22 | `src/application/navigation.rs` | L6 | `NavigationHistory` `VecDeque<String>` sem limite — cresce indefinidamente |

---

### BAIXO (15 issues)

| ID | Arquivo | Problema |
|----|---------|----------|
| L-1 | `src/infrastructure/directory_index.rs` L100 | `.filter_map(r.ok())` silenciosamente descarta rows corrompidos sem log |
| L-2 | `src/infrastructure/drive_watcher/thread_loop.rs` L42 | ~140 linhas em um único bloco `unsafe` (inclui safe Rust) |
| L-3 | `src/workers/thumbnail/mod.rs` L82 | Failure cache limpa tudo a 1000 entries em vez de eviction LRU |
| L-4 | `src/workers/file_operation_worker/handlers.rs` L20 | Resultado de `shell_operations::delete_items_with_shell()` ignorado (`let _`) |
| L-5 | `src/image_viewer/app.rs` L225 | GIF decode bloqueante na UI thread do image viewer standalone |
| L-6 | `src/pdf_viewer/webview.rs` L269 | COM ref counting usa `Ordering::Relaxed` (correto em STA, incorreto se multi-thread) |
| L-7 | `src/tabs/mod.rs` L240 | `active()` faz panic se tabs vazio (invariante não codificada no type system) |
| L-8 | `src/pdf_viewer/window.rs` L67 | URL percent-encoding incompleto para nomes de arquivo com caracteres especiais |
| L-9 | `src/video_player/mod.rs` L130 | Erros de propriedades mpv ignorados silenciosamente (`let _`) |
| L-10 | `src/application/watcher.rs` L31 | `request_auto_reload()` reseta timer a cada evento, adiando debounce indefinidamente durante bursts |
| L-11 | `src/application/sorting/sort_impl.rs` L6 | Parse de data da lixeira assume formato `dd/mm/yyyy` (quebra em locales US) |
| L-12 | `src/app/operations/context_menu.rs` L10 | `context_target_paths()` clona `Vec<PathBuf>` a cada invocação |
| L-13 | `src/app/operations/tabs.rs` L19 | `sync_to_tab()` usa `mem::take` deixando `all_items` vazio — race window |
| L-15 | `src/ui/app_impl.rs` L17 | `ctx.set_zoom_factor(1.0)` chamado todo frame (poderia ser condicional) |
| L-17 | `src/app/operations/context_menu.rs` L390 | `update_ui_item` clona subitem Vec em cada recursive call |

> **Nota:** L-14 foi promovido a **H-8** (blocking Shell calls durante drag viola R-1). L-16 era duplicata de M-11.

---

## 3. Oportunidades de Otimização e Performance

### 3.1 Alocações Per-Frame no egui hot path

**Problema:** O `update()` executa ~60 vezes/segundo. Múltiplas alocações de heap desnecessárias ocorrem a cada frame.

**Como otimizar — Clones em panels.rs (H-1):**
```rust
// ANTES (panels.rs L94-97) — aloca a cada frame
let disks = app.drive_state.disks.clone();
let current_path = app.navigation_state.current_path.clone();

// DEPOIS — zero allocations
render_sidebar(
    ui,
    &app.drive_state.disks,              // referência
    &app.navigation_state.current_path,  // &str
    app.cache_manager.computer_icon.as_ref(),
);
```

**Como otimizar — Breadcrumbs (M-3):**
```rust
// ANTES — recomputa path components todo frame
let components: Vec<_> = path.components().collect();

// DEPOIS — cache invalidado apenas quando current_path muda
struct BreadcrumbCache {
    source_path: String,
    segments: Vec<(String, String)>, // (display_name, full_path)
}
```

**Como otimizar — egui IDs com format! (M-8, M-9):**
```rust
// ANTES — allocation por notification por frame
egui::Id::new(format!("toast_{}", i))

// DEPOIS — zero allocation
egui::Id::new("toast").with(i)
```

**Como otimizar — Spinner Vec (M-10):**
```rust
// ANTES — heap Vec de 20 elementos
let points: Vec<egui::Pos2> = (0..20).map(|i| { ... }).collect();

// DEPOIS — stack array, zero heap
let points: [egui::Pos2; 20] = std::array::from_fn(|i| { ... });
```

**Como otimizar — Context menu partitioning (M-5):**
```rust
// ANTES — 3 Vec::collect() todo frame enquanto menu aberto
let primary: Vec<&ContextMenuItem> = items.iter().filter(|i| i.is_primary).collect();

// DEPOIS — pré-particionado ao abrir o menu
struct ContextMenuState {
    primary_items: Vec<ContextMenuItem>,
    secondary_items: Vec<ContextMenuItem>,
    overflow_items: Vec<ContextMenuItem>,
}
```

**Como otimizar — Status bar labels (M-13):**
```rust
// ANTES — format! todo frame
ui.label(format!("RAM: {}", format_size(ram_usage)));

// DEPOIS — cache string junto com o valor
struct CachedLabel { value: u64, formatted: String }
// Atualiza formatted apenas quando value muda (1x/segundo)
```

**Como otimizar — TRUNCATION_CACHE (M-6):**
```rust
// ANTES — HashMap com clear() total a 2000 entries (cache avalanche)
if cache.len() > 2000 { cache.clear(); }

// DEPOIS — LruCache com eviction suave (mesmo padrão do FONT_WIDTH_CACHE)
static TRUNCATION_CACHE: Lazy<Mutex<LruCache<u64, String>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(2000).unwrap())));
```

**Ganho estimado total:** Elimina ~300+ heap allocations/segundo em uso normal.

---

### 3.2 `Arc<Mutex<Receiver>>` nas Worker Pools (M-18)

**Problema:** Folder preview workers (2-6 threads) e icon workers (2-16 threads) compartilham um `Receiver` via `Arc<Mutex<...>>`. Apenas 1 thread pode esperar em `recv()` por vez.

**Como otimizar:**
```rust
// ANTES — Mutex serializa o dispatch
let rx = Arc::new(Mutex::new(receiver));

// DEPOIS — crossbeam MPMC permite espera concorrente
let (tx, rx) = crossbeam_channel::unbounded::<PathBuf>();
// Thread: while let Ok(path) = rx.recv()  // sem Mutex!
```

**Ganho:** Dispatch simultâneo para N workers em vez de serializado.

---

### 3.3 GC do SQLite com `ORDER BY RANDOM()` (M-15)

**Problema:** O GC incremental do thumbnail cache usa `ORDER BY RANDOM() LIMIT ?1`, forçando SQLite a score e ordenar a tabela inteira.

**Como otimizar:**
```sql
-- ANTES — O(n log n)
SELECT id, path FROM thumbnails ORDER BY RANDOM() LIMIT ?1

-- DEPOIS — O(1) amortizado
SELECT id, path FROM thumbnails
WHERE rowid >= (ABS(RANDOM()) % (SELECT MAX(rowid) FROM thumbnails))
LIMIT ?1
```

---

### 3.4 Drag Preview Icon Loading (L-14)

**Problema:** `render_item_drag_preview()` chama `get_or_load_icon` com `allow_blocking = true` a cada frame durante drag.

**Como otimizar:**
```rust
pub fn begin_item_drag(&mut self) {
    // ... existing code ...
    self.drag_icon_texture = self.get_icon_for_path(&path); // cache once
}
```

---

### 3.5 Metadata Refresh Throttling (M-2)

**Problema:** `refresh_selected_metadata()` é chamado todo frame com preview panel aberto, clonando path e fazendo lookups em cache mesmo sem mudança.

**Como otimizar:**
```rust
// ANTES — chamado todo frame
app.refresh_selected_metadata();

// DEPOIS — chamado apenas quando selected_file muda
if app.metadata_dirty {
    app.refresh_selected_metadata();
    app.metadata_dirty = false;
}
// Set metadata_dirty = true apenas em: seleção de arquivo, watcher event, tab switch
```

---

### 3.6 Tab Title Truncation Cache (M-4)

**Problema:** Binary search com `format!("{}...", ...)` a cada tab a cada frame para truncação de título.

**Como otimizar:**
```rust
struct CachedTabTitle {
    source: String,
    available_width: f32,
    truncated: String,
}
// Invalidar apenas quando tab.title ou width muda
```

---

## 4. Melhorias de Estabilidade

### 4.1 RAII Guards para Recursos OS

Criar wrappers RAII para `HANDLE`, `COM` e transações SQLite:

```rust
// Handle guard para ntfs_reader, drive_watcher, etc.
struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) { unsafe { let _ = CloseHandle(self.0); } }
}

// COM guard (já existe ComApartmentGuard — aplicar em folder_preview_worker)
// Transação SQLite (usar conn.transaction() em vez de raw SQL)
```

Aplicar em: `ntfs_reader.rs` (H-4), `folder_preview_worker.rs` (M-19), `visual_workers.rs`.

---

### 4.2 Transações SQLite com Auto-Rollback

Substituir todas as ocorrências de `BEGIN TRANSACTION`/`COMMIT` raw:

```rust
// ANTES (gc.rs L177-192)
let _ = db.execute("BEGIN TRANSACTION", []);
// ... batch operations ...
let _ = db.execute("COMMIT", []);

// DEPOIS — auto-rollback on Drop
let tx = db.transaction()?;
// ... batch operations ...
tx.commit()?;
```

Aplicar em: `gc.rs` (H-5), `cleanup.rs` (M-16).

---

### 4.3 Shutdown Gracioso de Workers

Em `handle_exit()`, dropar todos os senders para sinalizar workers:

```rust
fn handle_exit(&mut self) {
    self.thumbnail_queue.shutdown();
    // Dropar senders → workers detectam canal fechado via recv() -> Err
    drop(self.cover_worker_sender.take());
    drop(self.folder_preview_sender.take());
    drop(self.icon_req_sender.take());
    drop(self.metadata_req_sender.take());
    // GC worker: usar AtomicBool ou recv_timeout em vez de loop infinito
}
```

---

### 4.4 Bounds Check no Buffer Parser do Drive Watcher

```rust
// ANTES (buffer_parser.rs L28-36) — confia em kernel data
let name_slice = std::slice::from_raw_parts(name_ptr, name_len);

// DEPOIS — valida bounds
if offset + std::mem::size_of::<FILE_NOTIFY_INFORMATION>() + name_len * 2 > buffer.len() {
    log::warn!("RDCW buffer overflow detected, skipping entry");
    break;
}
let name_slice = std::slice::from_raw_parts(name_ptr, name_len);
```

---

### 4.5 Logging de Erros Silenciosos

Substituir `let _ = result` por logging em pontos críticos:

```rust
// Shell operation results (L-4)
if let Err(e) = shell_operations::delete_items_with_shell(...) {
    log::error!("[FILE-OP] Shell delete failed: {:?}", e);
    // enviar resultado de erro em vez de sucesso
}

// file_op_sender (H-3)
if self.file_operation_state.file_op_sender.send(req).is_err() {
    self.file_operation_state.file_ops_in_progress =
        self.file_operation_state.file_ops_in_progress.saturating_sub(1);
    log::error!("[FILE-OP] Worker channel disconnected");
}
```

---

### 4.6 NavigationHistory Bounded (M-22)

```rust
const MAX_HISTORY: usize = 500;
pub fn navigate_to(&mut self, path: String) {
    // ... existing truncation logic ...
    self.paths.push_back(path);
    if self.paths.len() > MAX_HISTORY {
        self.paths.pop_front();
        self.current_index = self.current_index.saturating_sub(1);
    }
    self.current_index = self.paths.len() - 1;
}
```

---

### 4.7 WebView2 Controller::Close() (H-7)

```rust
impl Drop for WebViewState {
    fn drop(&mut self) {
        unsafe {
            let vtbl = *(self.controller.ptr as *mut *mut ICoreWebView2Controller_Vtbl);
            // Close browser process ANTES de liberar COM refs
            ((*vtbl).Close)(self.controller.ptr);
            ((*vtbl).base.Release)(self.controller.ptr);
            let vtbl_wv = *(self.webview.ptr as *mut *mut ICoreWebView2_Vtbl);
            ((*vtbl_wv).base.Release)(self.webview.ptr);
        }
    }
}
```

---

### 4.8 Auto-reload Debounce Fix (L-10)

```rust
pub fn request_auto_reload(&mut self) {
    if !self.pending_auto_reload {
        self.last_auto_reload = Instant::now(); // só no primeiro evento
    }
    self.pending_auto_reload = true;
}
```

---

## 5. Padrões Positivos (Preservar)

Estes são padrões bem implementados que devem ser mantidos:

1. **Frame timing guards** (`app_impl.rs`): EWMA frame time tracking com `last_actual_frame_ms` para OS paging spikes
2. **Icon loading budget** (`icon_loader.rs`): Max 6 sync lookups ou 4ms por 16ms window
3. **Atomic TTL caching** (`status_bar.rs`): RAM/VRAM cached com 1s TTL via atomics — zero-allocation reads
4. **FxHashSet** (`list_view`, `cache.rs`): Hashing rápido para PathBuf keys
5. **Manual virtualization** (`grid_view/`, `list_view/`): Adaptive overscan (2 during scroll, 5 idle), HDD-aware prefetch
6. **Poison-safe Mutex** (`sidebar.rs`): `.unwrap_or_else(|e| e.into_inner())` — recuperação correta
7. **Event-driven metadata** (`metadata.rs`): Early return quando same file, DriveWatcher limpa `last_metadata_path`
8. **Throttled spinner** (`folder_slot.rs`): `request_repaint_after(66ms)` — 15 FPS suficiente para spinner
9. **No `path.exists()` in render loop** (`panels.rs`, `app_impl.rs`): Explicitamente comentado e evitado
10. **Pre-allocated PendingOperations** (`grid_view`): Vec buffers cleared e reutilizados em vez de realocados
11. **Generation-based stale detection** (thumbnail workers): Previne upload de thumbnails obsoletos
12. **Memory maintenance com working set monitoring**: Limites soft (550MB) / hard (700MB) com trim adaptativo
13. **`safe_unwrap!`/`safe_expect!` macros** (domain layer): Zero `.unwrap()` em production paths

---

## 6. Plano de Implementação (Roadmap)

> **Princípio de ordenação:** Este roadmap é priorizado por **requisitos de UX** (R-1, R-2, R-3), não por esforço. A regra é: **primeiro garantir zero freezes e zero placeholders, depois maximizar velocidade, depois polir.**

### Fase 1 — Zero Freezes na UI + Eliminação de Placeholders (R-1 + R-3) — ~4-5 dias

> **Objetivo:** Ao final desta fase, a UI **nunca congela** e **nenhum placeholder visual** existe na interface. Esta é a fase mais importante de todo o roadmap.

| # | Issue | Esforço | Impacto | Requisito |
|---|-------|---------|---------|-----------|
| 1 | **C-1:** Substituir `is_dir()` por `item.is_dir` no context menu | 5 min | Elimina freeze 5-60s no right-click | R-1 |
| 2 | **C-4:** Remover spinner + rect cinza do folder_slot.rs | 1h | Elimina placeholder visual em pastas | R-3 |
| 3 | **C-5:** Remover rect cinza do file_slot.rs | 30 min | Elimina placeholder visual em arquivos | R-3 |
| 4 | **C-7:** Remover rects cinza de fallback em caminhos especiais | 30 min | Elimina placeholders residuais em pastas | R-3 |
| 5 | **C-6:** Mover `extract_drive_icon()` para background thread | 2h | Elimina freeze 50-200ms/drive no "This PC" | R-1 + R-2 |
| 6 | **C-3:** `show_properties` assíncrono (dispatch para background) | 4h | Elimina freeze no Alt+Enter / Properties | R-1 |
| 7 | **C-2:** `extract_shell_menu` assíncrono | 1 dia | Elimina freeze 1-10s no right-click | R-1 |
| 8 | **H-8:** Cachear drag icon texture em `begin_item_drag` | 30 min | Elimina blocking Shell calls todo frame no drag | R-1 |

**Entregável:** UI 100% non-blocking. Nenhuma imagem temporária, spinner, ou retângulo cinza na interface.

---

### Fase 2 — Velocidade Máxima de Carregamento de Mídia (R-2) — ~2-3 dias

> **Objetivo:** Workers de thumbnail/icon/preview operam em throughput máximo. Ícones e thumbnails aparecem o mais rápido possível na interface.

| # | Issue | Esforço | Impacto | Requisito |
|---|-------|---------|---------|-----------|
| 9 | **M-18:** Migrar `Arc<Mutex<Receiver>>` → crossbeam-channel MPMC | 2h | Workers em paralelo real (sem serialização) | R-2 |
| 10 | **M-17:** Separar reader/writer no disk_cache (eliminar stall) | 2h | Reads de thumbnail sem bloqueio durante GC | R-2 |
| 11 | **M-15:** Fix `ORDER BY RANDOM()` no GC → ROWID modular | 30 min | GC O(1) em vez de O(n log n) — menos interferência | R-2 |
| 12 | **M-20:** Overlapped I/O no global_search pipe | 1 dia | Elimina busy-wait CPU no search | R-2 |
| 13 | **L-5:** GIF decode assíncrono no image viewer standalone | 3h | Elimina freeze ao abrir GIFs pesados | R-1 + R-2 |

**Entregável:** Thumbnails, ícones e previews carregam na velocidade máxima que o hardware permite.

---

### Fase 3 — Fluidez Visual: Per-frame Allocations (R-1) — ~2-3 dias

> **Objetivo:** Suavidade de 60 FPS consistente. Eliminar todas as alocações de heap desnecessárias no hot path do `update()`.

| # | Issue | Esforço | Impacto |
|---|-------|---------|---------|
| 14 | **H-1:** Eliminar clones per-frame em panels.rs (disks, path, selected_file) | 3h | -240 allocations/s |
| 15 | **M-2:** Metadata refresh com dirty flag (evitar probes todo frame) | 1h | Elimina path clone + cache probes/frame |
| 16 | **M-3:** Cache de breadcrumb segments no toolbar.rs | 1h | Elimina path parsing + allocs todo frame |
| 17 | **M-4:** Cache de tab titles truncados | 1h | Elimina binary search + format!/frame |
| 18 | **M-5:** Pré-particionar context menu items ao abrir (não por frame) | 30 min | Elimina 3× Vec::collect/frame |
| 19 | **M-6:** TRUNCATION_CACHE → LruCache (como FONT_WIDTH_CACHE) | 30 min | Elimina cache avalanche periódico |
| 20 | **M-7:** SVG cache key: hash em vez de `to_string()` | 30 min | -1 String alloc por icon lookup |
| 21 | **M-8/M-9:** `format!()` → `Id::new().with()` em toast/tabs | 15 min | -4 allocations/frame |
| 22 | **M-11:** Reutilizar Vec para char_boundaries (stack ou pre-allocated) | 15 min | Elimina alloc por cache miss |
| 23 | **M-12:** Cache "current directory" FileEntry quando sem seleção | 1h | Elimina Path/String allocs |
| 24 | **M-13:** Cache formatted RAM/VRAM labels (1s TTL) | 30 min | -2-3 format!/frame |

**Entregável:** Zero alocações de heap desnecessárias no render loop. Frame time consistente ≤16ms.

---

### Fase 4 — Estabilidade e Integridade de Dados — ~3-4 dias

> **Objetivo:** Robustez: recursos liberados corretamente, dados não corrompidos, workers encerrados com graça.

| # | Issue | Esforço | Impacto |
|---|-------|---------|---------|
| 25 | **H-3:** Checar resultado de `file_op_sender.send()` | 30 min | Previne indicador permanente de "em progresso" |
| 26 | **H-2:** Cleanup de placeholder `.lnk` em error path | 15 min | Previne arquivos lixo no disco |
| 27 | **H-4:** RAII guard para HANDLE no ntfs_reader | 1h | Memory safety on panic |
| 28 | **H-5:** Transações SQLite com auto-rollback (gc.rs) | 2h | Integridade do cache de thumbnails |
| 29 | **H-6:** Shutdown gracioso de workers (dropar senders) | 3h | Clean exit, sem data corruption |
| 30 | **H-7:** `Controller::Close()` no PDF viewer antes de Release | 30 min | Elimina processos Edge órfãos |
| 31 | **M-14:** Guard `i64 as u64` com `.max(0)` no NTFS reader | 5 min | Previne tamanhos absurdos |
| 32 | **M-16:** Wrap DELETEs do cleanup.rs em transação | 30 min | Performance + atomicidade |
| 33 | **M-19:** COM RAII no folder_preview_worker | 30 min | Consistência com o resto do codebase |
| 34 | **M-21:** Bounds check no buffer parser | 30 min | Previne UB com kernel data corrompida |
| 35 | **M-22:** Cap `NavigationHistory` em 500 entries | 15 min | Previne memory leak em sessões longas |
| 36 | **L-4:** Checar resultado de `delete_items_with_shell()` | 15 min | Previne falsa notificação de sucesso |
| 37 | **L-10:** Fix debounce de `request_auto_reload()` | 5 min | Reload mais responsivo |
| 38 | **L-1:** Log warning em rows corrompidas do directory_index | 5 min | Visibilidade diagnóstica |

**Entregável:** Sem resource leaks, sem data corruption, shutdown limpo.

---

### Fase 5 — Polish & Hardening — ongoing

> **Objetivo:** Limpeza de code smells, edge cases, e minor inefficiencies.

| # | Issue | Esforço |
|---|-------|---------|
| 39 | **L-2:** Narrowar blocos unsafe no thread_loop.rs | 1h |
| 40 | **L-3:** LRU eviction no failure cache de thumbnails | 1h |
| 41 | **L-6:** Fix COM ref counting atomic ordering (Relaxed → Release/Acquire) | 30 min |
| 42 | **L-7:** Encode invariante "tabs não vazio" no type system | 30 min |
| 43 | **L-8:** URL percent-encoding completo no PDF viewer | 30 min |
| 44 | **L-9:** Log erros de propriedades mpv | 15 min |
| 45 | **L-11:** Data lixeira: preferir timestamp numérico em vez de parse locale | 30 min |
| 46 | **L-12:** `context_target_paths()` retornar `&[PathBuf]` em vez de clonar | 15 min |
| 47 | **L-13:** `sync_to_tab()` — avaliar alternativa a `mem::take` | 30 min |
| 48 | **L-15:** `set_zoom_factor` condicional | 5 min |
| 49 | **L-17:** `update_ui_item` evitar clone recursivo de subitems | 30 min |

---

## Resumo por Severidade

| Severidade | Count | Temas Principais |
|------------|-------|-----------------|
| **Crítico** | 7 | Blocking I/O na UI thread (R-1), placeholders visuais proibidos (R-3) |
| **Alto** | 8 | Per-frame clones, resource leaks, shutdown incompleto, DB integrity, drag blocking |
| **Médio** | 20 | Per-frame allocations (egui), worker throughput (R-2), SQLite perf |
| **Baixo** | 15 | Minor inefficiencies, logging gaps, edge cases |
| **Total** | **50** | |

### Alinhamento com Requisitos

| Requisito | Issues Diretamente Relacionadas | Fase Principal |
|-----------|--------------------------------|----------------|
| **R-1** (Zero freezes) | C-1, C-2, C-3, C-6, H-8, L-5 | Fase 1 |
| **R-2** (Velocidade máxima) | C-6, M-17, M-18, M-20, L-5 | Fase 2 |
| **R-3** (Zero placeholders) | C-4, C-5, C-7 | Fase 1 |

**Nota final:** A base de código está significativamente acima da média para um projeto Rust de ~50K linhas. Com os requisitos R-1/R-2/R-3 em vigor, a Fase 1 do roadmap (zero freezes + zero placeholders) é a **prioridade absoluta** e deve ser completada antes de qualquer outro trabalho. As issues C-1 (5 min), C-4, C-5, C-7 (~2h total) são resolvíveis rapidamente; C-2 (1 dia) e C-6 (2h) requerem mais trabalho mas têm impacto direto na experiência do usuário.
