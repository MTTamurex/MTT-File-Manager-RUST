# Plano de Refatoração - Arquivos Monolito

> **Gerado em:** 2026-02-02  
> **Analisado por:** Agente de Análise de Código  
> **Status:** Aguardando execução

---

## Resumo Executivo

Este documento contém o plano detalhado de refatoração para 5 arquivos monolito identificados no projeto MTT File Manager.

| Arquivo | Linhas | Score | Prioridade |
|---------|--------|-------|------------|
| `ui/preview_panel.rs` | 1.962 | 🔴 CRÍTICO | P0 - Iniciar primeiro |
| `ui/views/list_view.rs` | 1.415 | 🔴 ALTO | P1 - Segundo |
| `workers/thumbnail_worker.rs` | 1.259 | 🟡 MÉDIO-ALTO | P2 |
| `app/operations/ui_rendering.rs` | 1.106 | 🟡 MÉDIO | P3 |
| `ui/components/mpv_preview.rs` | 1.030 | 🟡 MÉDIO | P4 |

---

## 🔴 P0: `src/ui/preview_panel.rs` (1.962 linhas)

### Problemas Identificados

1. **Função `render_preview_panel`: ~900 linhas** - Controla TODA a lógica do painel
2. **Múltiplas responsabilidades misturadas:**
   - Preview de imagens estáticas
   - Preview de vídeos (3 modos: docked, detached, fullscreen)
   - Preview de GIFs animados
   - Preview de pastas (com folder peek)
   - Preview de drives
   - Tabela de metadados (EXIF, codecs, etc.)
   - Controles de vídeo (play/pause, seek, volume)
3. **Código duplicado:** Controles de vídeo aparecem em 3 lugares diferentes
4. **Complexidade:** ~15 closures/funções internas grandes

### Estrutura Alvo

```
src/ui/preview_panel/
├── mod.rs                    # Coordenação principal (~200 linhas)
│   └── render_preview_panel() - delega para sub-módulos
├── actions.rs                # PreviewPanelAction enum
├── file_info_table.rs        # Tabela de metadados
│   └── Extrair de: render_preview_panel (~300 linhas)
├── image_preview.rs          # Preview de imagens estáticas
│   └── render_image_preview()
├── video_preview/
│   ├── mod.rs               # Coordenação e detecção
│   ├── docked.rs            # Modo anexado (docked)
│   ├── detached.rs          # Janela flutuante
│   ├── fullscreen.rs        # Tela cheia
│   └── controls.rs          # Barra de controles (ÚNICA - reuso)
├── folder_preview.rs         # Preview de pastas com folder_peek
├── drive_preview.rs          # Preview de drives
└── fallback_renderer.rs      # Fallback de ícones do sistema
```

### Funções a Extrair

| Função Original | Linhas | Destino | Notas |
|----------------|--------|---------|-------|
| `render_preview_panel` | ~900 | `mod.rs` (~150) + módulos | Delegar para sub-módulos |
| `draw_controls` (closure) | ~250 | `video_preview/controls.rs` | Reusar em todos os modos |
| `render_fallback` | ~200 | `fallback_renderer.rs` | Lógica de fallback complexa |
| `render_texture_with_overlay` | ~120 | `image_preview.rs` | Overlays específicos |
| `render_file_info_table` | ~300 | `file_info_table.rs` | Metadados EXIF, etc. |

### Critérios de Aceitação

- [ ] Nenhuma função >150 linhas no módulo principal
- [ ] Controles de vídeo em único lugar (sem duplicação)
- [ ] Cada tipo de preview isolado em seu arquivo
- [ ] Testar: preview de imagem, vídeo, GIF, pasta, drive
- [ ] Verificar: seleção múltipla ainda funciona

### Esforço Estimado

**2-3 dias** de trabalho focado.

---

## 🔴 P1: `src/ui/views/list_view.rs` (1.415 linhas)

### Problemas Identificados

1. **`ListViewContext` com 25+ campos** - Tudo junto, sem agrupamento
2. **`render_list_view`: ~650 linhas** - 3 modos diferentes misturados
3. **`render_list_item`: ~450 linhas** - Trata normal, computer view, OneDrive
4. **Responsabilidades misturadas:**
   - Virtualização de lista (scroll manual)
   - Renderização de cabeçalhos com resize
   - Renderização de itens com lazy loading
   - 3 modos de visualização (normal, computer, OneDrive)
   - Lógica de renomeação inline
   - Tooltips com debounce

### Estrutura Alvo

```
src/ui/views/list_view/
├── mod.rs                   # ~150 linhas - coordenação
│   └── render_list_view() - delega para sub-funções
├── context.rs               # ListViewContext refactorado
│   └── Agrupar em sub-structs:
│       - SelectionContext
│       - ScrollContext
│       - ColumnContext
│       - CacheContext
├── virtualization.rs        # Lógica de scroll/virtualização
│   └── Cálculo de vis_min_row, vis_max_row, etc.
├── header.rs                # Cabeçalhos com resize handles
│   └── draw_header_resizable() extraído
├── item_renderer.rs         # Renderização de cada linha
│   └── render_list_item() simplificado
├── modes/
│   ├── mod.rs              # Enum ViewMode
│   ├── normal.rs           # Visualização normal
│   ├── computer_view.rs    # Visualização "Este Computador"
│   └── onedrive.rs         # Coluna de status OneDrive
├── rename.rs                # Lógica de renomeação inline
└── tooltip.rs               # Tooltip com debounce
```

### Context Refactoring

```rust
// ANTES: 25+ campos soltos
pub struct ListViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub multi_selection: &'a FxHashSet<PathBuf>,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub renaming_state: Option<(usize, String)>,
    pub focus_rename: bool,
    pub scroll_to_selected: bool,
    pub is_computer_view: bool,
    pub is_recycle_bin_view: bool,
    pub is_onedrive_folder: bool,
    pub texture_cache: &'a mut LruCache<PathBuf, egui::TextureHandle>,
    // ... mais 15 campos
}

// DEPOIS: Agrupado por responsabilidade
pub struct ListViewContext<'a> {
    pub items: &'a [FileEntry],
    pub selection: SelectionContext<'a>,
    pub scroll: ScrollContext<'a>,
    pub columns: ColumnContext<'a>,
    pub cache: CacheContext<'a>,
    pub view_mode: ViewMode,  // Normal | Computer | OneDrive
}

pub struct SelectionContext<'a> {
    pub selected_item: Option<usize>,
    pub selected_file: Option<&'a FileEntry>,
    pub multi_selection: &'a FxHashSet<PathBuf>,
    pub anchor: Option<usize>,
}

pub struct ScrollContext<'a> {
    pub offset_y: f32,
    pub mut_offset_y: &'a mut f32,
    pub to_selected: bool,
    pub last_time: &'a mut Instant,
    pub last_offset: &'a mut f32,
}
```

### Funções a Extrair

| Função Original | Linhas | Destino |
|----------------|--------|---------|
| `render_list_view` | ~650 | `mod.rs` (~150) + sub-módulos |
| `render_list_item` | ~450 | `item_renderer.rs` (~150 cada modo) |
| `draw_header_resizable` | ~90 | `header.rs` |
| Renomeação inline | ~80 | `rename.rs` |
| Tooltip | ~50 | `tooltip.rs` |

### Critérios de Aceitação

- [ ] ListViewContext com máximo 8 campos no nível raiz
- [ ] Cada modo de visualização isolado
- [ ] Scroll/virtualização separado de renderização
- [ ] Testar: scroll, seleção, renomeação, sorting
- [ ] Testar: alternar entre modos (normal <-> computer <-> OneDrive)

### Esforço Estimado

**1-2 dias** de trabalho focado.

---

## 🟡 P2: `src/workers/thumbnail_worker.rs` (1.259 linhas)

### Problemas Identificados

1. **Pipeline de 5 estágios** em um único arquivo
2. **`thumbnail_worker_loop`: ~280 linhas** - Loop principal denso
3. **`try_media_foundation_extraction`: ~220 linhas** - Extração de vídeo complexa
4. **Responsabilidades misturadas:**
   - Fila de prioridade (PriorityThumbnailQueue)
   - Worker threads e concorrência
   - Pipeline de extração híbrida (5 estágios)
   - Processamento de imagem (resize, conversão NV12)
   - APIs Windows (WIC, Shell API, Media Foundation)

### Estrutura Alvo

```
src/workers/thumbnail/
├── mod.rs                   # Fila e coordenação (~150 linhas)
├── queue.rs                 # PriorityThumbnailQueue
│   └── QueueState, PriorityThumbnailQueue
├── worker.rs                # Thread pool e loop simplificado
│   └── spawn_thumbnail_workers(), thumbnail_worker_loop() (~100 linhas)
├── extraction/              
│   ├── mod.rs              # generate_thumbnail_hybrid() - coordena estágios
│   ├── stage1_image_crate.rs   # try_image_crate_extraction()
│   ├── stage2_wic.rs           # try_wic_extraction()
│   ├── stage3_shell_api.rs     # extract_windows_thumbnail_shell()
│   ├── stage4_force_extract.rs # force_extract_thumbnail()
│   └── stage5_media_foundation.rs  # try_media_foundation_extraction()
├── processing/
│   ├── mod.rs
│   ├── resize.rs           # resize_to_bucket(), get_bucket_size()
│   └── format_conversion.rs # convert_nv12_to_rgba()
├── cache_integration.rs     # Interface com disk_cache
└── types.rs                 # ThumbnailRequest, ThumbnailPriority
```

### Pipeline de Extração

```rust
// extraction/mod.rs
pub fn generate_thumbnail_hybrid(
    path: &Path,
    priority: IOPriority,
) -> Option<(Vec<u8>, u32, u32)> {
    // Stage 1: image crate (Fast Path)
    if let Some(result) = stage1_image_crate::extract(path, priority) {
        return Some(result);
    }

    // Stage 2: WIC (Robust Fallback)
    if let Some(result) = stage2_wic::extract(path) {
        return Some(result);
    }

    // Stage 3: Shell API (Universal/Video)
    if let Some(result) = stage3_shell_api::extract(path) {
        return Some(result);
    }

    // Stage 4: Force Extraction
    if let Some(result) = stage4_force_extract::extract(path) {
        return Some(result);
    }

    // Stage 5: Media Foundation (Nuclear Option)
    stage5_media_foundation::extract(path)
}
```

### Funções a Extrair

| Função Original | Linhas | Destino |
|----------------|--------|---------|
| `thumbnail_worker_loop` | ~280 | `worker.rs` (~100) + delegação |
| `generate_thumbnail_hybrid` | ~60 | `extraction/mod.rs` |
| `try_media_foundation_extraction` | ~220 | `extraction/stage5_media_foundation.rs` |
| `try_wic_extraction` | ~60 | `extraction/stage2_wic.rs` |
| `extract_windows_thumbnail_shell` | ~55 | `extraction/stage3_shell_api.rs` |
| `convert_nv12_to_rgba` | ~40 | `processing/format_conversion.rs` |
| `resize_to_bucket` | ~35 | `processing/resize.rs` |

### Critérios de Aceitação

- [ ] Cada estágio de extração isolado e testável
- [ ] Worker loop <100 linhas (delegação clara)
- [ ] Fácil adicionar novo estágio de extração
- [ ] Testar: extração de imagem, vídeo, formatos especiais

### Esforço Estimado

**1-2 dias** de trabalho focado.

---

## 🟡 P3: `src/app/operations/ui_rendering.rs` (1.106 linhas)

### Problemas Identificados

1. **Duplicação de código** entre list_view e grid_view:
   - Keyboard navigation (Arrow keys, Page Up/Down)
   - Lógica de seleção (Ctrl, Shift)
   - Integração com workers
2. **Funções grandes:**
   - `render_list_view` (impl): ~500 linhas
   - `render_grid_view` (impl): ~460 linhas
3. **Bridge complexa** entre App e Views

### Estrutura Alvo

```
src/app/operations/
├── navigation/              # NOVO MÓDULO
│   ├── mod.rs
│   ├── keyboard.rs         # Keyboard navigation (reuso)
│   │   └── handle_arrow_keys(), handle_page_keys()
│   └── selection.rs        # Multi-selection logic (reuso)
│       └── handle_ctrl_click(), handle_shift_click()
├── ui_rendering/
│   ├── mod.rs              # Coordenação (~100 linhas)
│   ├── list_bridge.rs      # Lógica específica de list
│   │   └── render_list_view() simplificado
│   └── grid_bridge.rs      # Lógica específica de grid
│       └── render_grid_view() simplificado
└── shared_actions.rs       # Ações compartilhadas
```

### Refactoring de Navegação

```rust
// navigation/keyboard.rs
pub fn handle_keyboard_navigation(
    app: &mut ImageViewerApp,
    ui: &mut egui::Ui,
    view_type: ViewType,  // List | Grid
) -> Option<NavigationAction> {
    // Lógica comum de keyboard navigation
    // Retorna ação a ser executada
}

// navigation/selection.rs
pub fn handle_selection(
    app: &mut ImageViewerApp,
    index: usize,
    modifiers: &egui::Modifiers,
) -> SelectionResult {
    // Lógica comum de seleção (Ctrl, Shift)
}
```

### Funções a Extrair

| Função Original | Linhas | Destino |
|----------------|--------|---------|
| Keyboard navigation (list) | ~80 | `navigation/keyboard.rs` |
| Keyboard navigation (grid) | ~80 | `navigation/keyboard.rs` (reuso) |
| Selection logic (list) | ~60 | `navigation/selection.rs` |
| Selection logic (grid) | ~60 | `navigation/selection.rs` (reuso) |
| `render_list_view` (impl) | ~500 | `ui_rendering/list_bridge.rs` (~200) |
| `render_grid_view` (impl) | ~460 | `ui_rendering/grid_bridge.rs` (~200) |

### Critérios de Aceitação

- [ ] Código de navegação sem duplicação
- [ ] Bridge simplificada (<200 linhas cada)
- [ ] Fácil adicionar novo modo de view
- [ ] Testar: navegação por teclado em ambos os modos
- [ ] Testar: seleção (Ctrl, Shift) em ambos os modos

### Esforço Estimado

**1 dia** de trabalho focado.

---

## 🟡 P4: `src/ui/components/mpv_preview.rs` (1.030 linhas)

### Problemas Identificados

1. **`update`: ~250 linhas** - Renderização + inicialização + polling
2. **Múltiplas responsabilidades:**
   - Integração com libmpv (comandos, eventos)
   - Gestão de janela nativa (HWND, posicionamento)
   - Filtros de vídeo (VSR, deinterlace, downscale docked)
   - Estado de playback (play/pause, seek, volume, tracks)
   - Thread de eventos assíncrona

### Estrutura Alvo

```
src/ui/components/mpv/
├── mod.rs                   # MpvPreview simplificado (~150 linhas)
├── state.rs                 # MpvState, TrackInfo
│   └── Structs de estado
├── mpv_bindings.rs          # Interface com libmpv
│   └── Comandos MPV (play, pause, seek, etc.)
├── window_management.rs     # HWND, posicionamento
│   ├── create_window()
│   ├── position_docked()
│   ├── position_detached()
│   └── set_fullscreen()
├── filters/
│   ├── mod.rs
│   ├── docked.rs           # Downscale, FPS limit para docked
│   ├── vsr.rs              # NVIDIA VSR
│   └── deinterlace.rs      # Detecção e filtros
├── playback/
│   ├── mod.rs
│   ├── controls.rs         # Play, pause, seek, volume
│   └── tracks.rs           # Audio/subtitle track selection
├── event_loop.rs            # Async polling thread
└── utils.rs                 # Funções utilitárias
```

### Funções a Extrair

| Função Original | Linhas | Destino |
|----------------|--------|---------|
| `update` | ~250 | `mod.rs` (~100) + sub-módulos |
| `update_docked_downscale` | ~120 | `filters/docked.rs` |
| `start_event_loop` | ~80 | `event_loop.rs` |
| Comandos MPV | ~60 | `mpv_bindings.rs` |
| Track management | ~50 | `playback/tracks.rs` |

### Critérios de Aceitação

- [ ] MpvPreview <150 linhas
- [ ] Filtros isolados e testáveis
- [ ] Window management separado de playback
- [ ] Testar: docked, detached, fullscreen
- [ ] Testar: filtros (VSR, deinterlace)

### Esforço Estimado

**1-2 dias** de trabalho focado.

---

## Checklist Geral de Refatoração

### Para cada arquivo:

- [ ] Criar estrutura de pastas/arquivos
- [ ] Extrair código para novos módulos
- [ ] Atualizar imports no resto do projeto
- [ ] Verificar compilação: `cargo check`
- [ ] Testar funcionalidades afetadas
- [ ] Verificar performance (não deve regredir)

### Métricas de Sucesso

- Nenhuma função >150 linhas (ideal: <100)
- Máximo 3 responsabilidades por módulo
- Cobertura de testes onde possível
- Documentação inline para APIs públicas

---

## Comandos Úteis

```bash
# Verificar tamanho dos arquivos após refatoração
cd src && find . -name "*.rs" -exec wc -l {} + | sort -rn | head -20

# Verificar compilação
cargo check
cargo clippy

# Testar
cargo test

# Build de release para testar performance
cargo build --release
```

---

## Notas para o Agente de Refatoração

1. **Comece pelo P0** (`preview_panel.rs`) - maior impacto
2. **Mantenha compatibilidade** - não altere APIs públicas sem necessidade
3. **Teste frequentemente** - compile e teste a cada extracao
4. **Commits atômicos** - um commit por módulo extraído
5. **Documente mudanças** - atualize este arquivo com progresso

---

**Próximo passo:** Iniciar refatoração de `preview_panel.rs` (P0).
