# Relatório de Otimizações de Performance UI/Gráfica

## MTT File Manager — Análise Profunda do Código-Fonte

**Data:** Fevereiro 2026
**Escopo:** Performance gráfica, responsividade da UI, alocações em hot paths
**Restrição:** Nenhuma otimização pode degradar a experiência do usuário (qualidade de thumbnails, velocidade de carregamento, comportamento de scroll, etc.)

---

## Arquitetura Atual (Contexto)

O MTT File Manager é um **file manager nativo em Rust** usando **egui 0.31** (GUI immediate-mode). Não é uma aplicação web/Tauri — renderiza diretamente via GPU com eframe.

### Pipeline de Renderização
```
┌──────────────────────────────────┐
│   egui update() — 60 FPS        │
│   (immediate-mode, frame-based)  │
└──────────────┬───────────────────┘
               │
       ┌───────┴───────┐
       │               │
   Grid View       List View
   (manual         (manual
    virtualization)  virtualization)
       │               │
       └───────┬───────┘
               │
       ┌───────▼────────────────┐
       │ Item Rendering         │
       │ (Icons, Text, Hover)   │
       └───────┬────────────────┘
               │
       ┌───────▼────────────────┐
       │ Cache System (3 camadas)│
       │ • VRAM (300 textures)  │
       │ • RAM (400 RGBA)       │
       │ • Disco (SQLite/WebP)  │
       └───────┬────────────────┘
               │
       ┌───────▼────────────────┐
       │ Worker Threads (4)     │
       │ • Thumbnail Workers    │
       │ • Icon Loader          │
       │ • Metadata Reader      │
       │ • Cover Worker         │
       └────────────────────────┘
```

### Métricas Atuais (código existente)
- Frame time tracking: média (EMA 90/10) + pico (decay 95%)
- Upload budget adaptativo: 2-10ms por frame baseado em FPS
- GPU uploads throttled: 1-12 por frame dependendo do estado

---

## TIER S — IMPACTO ALTO, COMPLEXIDADE SIMPLES

*Implementar primeiro. Retorno máximo com mínimo esforço.*

---

### S1. Remover `eprintln!` de hot paths no release

**Impacto: ALTO | Complexidade: SIMPLES | Degrada UX: NÃO**

O arquivo `message_handler.rs` contém **16 chamadas `eprintln!`** que executam em release mode. No Windows, `eprintln!` escreve para stderr de forma síncrona. Quando um console está anexado (ou quando o sistema está ocupado com I/O), cada chamada pode bloquear a thread UI por microsegundos. Em loops que processam eventos de watcher, isso se acumula.

**Evidência no código:**
- [message_handler.rs:165](src/app/operations/message_handler.rs#L165): `eprintln!("[RECYCLE] Operation finished, refreshing view.");`
- [message_handler.rs:436](src/app/operations/message_handler.rs#L436): `eprintln!("[FS-WATCH] CREATE: {:?}", ...);`
- [message_handler.rs:453](src/app/operations/message_handler.rs#L453): `eprintln!("[FS-WATCH] DELETE: {:?}", ...);`
- [message_handler.rs:504](src/app/operations/message_handler.rs#L504): `eprintln!("[FS-WATCH] MODIFY: {:?}", ...);`
- [message_handler.rs:626](src/app/operations/message_handler.rs#L626): `eprintln!("[DEBUG] Skipping auto-reload...");`
- E mais ~11 ocorrências no mesmo arquivo
- Também presente em [app_impl.rs:340](src/ui/app_impl.rs#L340): `eprintln!("[VIEW-MODE] Toolbar toggle...")`

**Passos de implementação:**
1. Criar macro `debug_log!` que compila para `eprintln!` apenas em `cfg(debug_assertions)`:
```rust
#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => { eprintln!($($arg)*) }
}
#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {}
}
```
2. Substituir todos os `eprintln!` em `message_handler.rs` por `debug_log!`
3. Verificar outros arquivos com `eprintln!` em hot paths

**Arquivos afetados:**
- `src/app/operations/message_handler.rs` (16 ocorrências)
- `src/ui/app_impl.rs` (1 ocorrência)

---

### S2. Eliminar `to_lowercase().ends_with(".zip")` repetido

**Impacto: ALTO | Complexidade: SIMPLES | Degrada UX: NÃO**

A verificação `.to_lowercase().ends_with(".zip")` aparece **15+ vezes** em hot paths de rendering. Cada chamada `to_lowercase()` aloca uma nova `String`. No grid view, isso executa por item visível por frame (60x/segundo). Para 80 itens visíveis, são 80+ alocações de String por frame — sem necessidade.

**Evidência no código — cada um chamado por item por frame:**
- [item_slot.rs:82](src/ui/components/item_slot.rs#L82): `ctx.item.name.to_lowercase().ends_with(".zip")` (render dispatch)
- [grid_view.rs:348](src/ui/views/grid_view.rs#L348): tooltip hover check
- [item_renderer.rs:268](src/ui/views/list_view/item_renderer.rs#L268): tooltip do list view
- [item_renderer.rs:331](src/ui/views/list_view/item_renderer.rs#L331): render do ícone no list view
- [item_renderer.rs:495](src/ui/views/list_view/item_renderer.rs#L495): coluna de tamanho no list view

**O `sorting.rs` já tem a solução correta** (`ends_with_ignore_case` sem alocação):
```rust
// sorting.rs:7-16
fn ends_with_ignore_case(s: &str, suffix: &str) -> bool {
    if s.len() < suffix.len() { return false; }
    let start = s.len() - suffix.len();
    s.as_bytes()[start..]
        .iter()
        .zip(suffix.as_bytes())
        .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
}
```

**Passos de implementação:**
1. Mover `ends_with_ignore_case` para `domain/file_entry.rs` como função pública
2. Adicionar método `is_zip(&self) -> bool` em `FileEntry`:
```rust
pub fn is_zip(&self) -> bool {
    ends_with_ignore_case(&self.name, ".zip")
}
```
3. Substituir todas as 15+ ocorrências por `item.is_zip()`
4. Opcionalmente, pré-computar `is_zip` como campo `bool` no `FileEntry` (calculado uma vez durante enumeração do diretório)

**Arquivos afetados:**
- `src/domain/file_entry.rs` (adicionar método)
- `src/ui/components/item_slot.rs`
- `src/ui/views/grid_view.rs`
- `src/ui/views/list_view/item_renderer.rs`
- `src/ui/views/common.rs`
- `src/ui/preview_panel/fallback_renderer.rs`
- `src/ui/preview_panel/file_info_table.rs`

---

### S3. Eliminar `path.clone()` desnecessário em `render_file_slot`

**Impacto: ALTO | Complexidade: SIMPLES | Degrada UX: NÃO**

`render_file_slot` em `item_slot.rs` faz `let path_clone = item.path.clone()` na **linha 457**, mas `item` já vive por toda a função via `ctx.item`. O `path_clone` é usado para lookups de cache que aceitam `&PathBuf`. O clone só é necessário nos raros casos onde o item precisa começar a carregar (uma vez, não por frame).

**Evidência:**
```rust
// item_slot.rs:457
let path_clone = item.path.clone();  // ALOCAÇÃO DESNECESSÁRIA POR ITEM POR FRAME

// Usos como &path_clone (leitura apenas — não precisa de ownership):
let has_texture = ctx.texture_cache.contains(&path_clone);  // L468
let is_loading = ctx.loading_set.contains(&path_clone);     // L469
let is_failed = ctx.failed_thumbnails.contains(&path_clone); // L470
let is_pending_upload = ctx.pending_upload_set.contains(&path_clone); // L471
```

**Passos de implementação:**
1. Remover `let path_clone = item.path.clone();` na linha 457
2. Usar `&item.path` para todas as chamadas `contains()`
3. Fazer clone apenas nos pontos que precisam de ownership:
   - `loading_set.insert(item.path.clone())` — só executa quando item começa a carregar
   - `ops.request_thumbnail_load(item.path.clone(), ...)` — só executa uma vez por item
4. Aplicar o mesmo padrão em `render_drive_slot` ([item_slot.rs:97](src/ui/components/item_slot.rs#L97))

**Arquivos afetados:**
- `src/ui/components/item_slot.rs` (linhas 97, 457)

---

## TIER A — IMPACTO ALTO, COMPLEXIDADE MODERADA

*Alto retorno mas requer mais cuidado na implementação.*

---

### A1. Cache de truncação de texto por (texto, largura) no list view

**Impacto: ALTO | Complexidade: MODERADA | Degrada UX: NÃO**

`truncate_text_for_column` é chamado para CADA item visível CADA frame, para CADA coluna (nome, data, tipo). Para 50 itens visíveis com 3 colunas, são ~150 chamadas por frame. Cada chamada faz binary search com múltiplas medições de texto via `get_cached_text_width`.

O texto e a largura da coluna **não mudam entre frames** (a não ser que a coluna seja redimensionada). Cachear o resultado eliminaria ~95% das computações de truncação.

**Evidência:**
```rust
// item_renderer.rs:184 — POR ITEM POR FRAME
let display_name = truncate_text_for_column(&item.name, available_name_width, &font_id, ui);
// item_renderer.rs:471 — POR ITEM POR FRAME
let display_date = truncate_text_for_column(&date_str, available_date_width, &font_id, ui);
// item_renderer.rs:484 — POR ITEM POR FRAME
let display_type = truncate_text_for_column(&type_str, available_type_width, &font_id, ui);
```

**Passos de implementação:**
1. Criar thread-local cache `HashMap<u64, String>` onde a chave é hash de `(texto, largura_como_bits)`
2. No início de `truncate_text_for_column`, computar hash e verificar cache
3. Se hit, retornar clone da String cacheada (barato vs. binary search + layout)
4. Invalidar cache quando colunas são redimensionadas (detectar mudança de `col_widths`)
5. Limitar cache a ~2000 entradas com mesma estratégia de clear

**Arquivos afetados:**
- `src/ui/views/list_view/mod.rs` (função `truncate_text_for_column`)

---

### A2. Chave do cache de largura de fonte aloca String desnecessariamente

**Impacto: ALTO | Complexidade: MODERADA | Degrada UX: NÃO**

`get_cached_text_width` cria `let key = (text.to_string(), font_id.size as u32, color)` a CADA chamada. Mesmo quando o texto está no cache, uma `String` é alocada para a lookup. Para ~150 chamadas por frame, são 150 alocações de String desperdiçadas.

**Evidência:**
```rust
// list_view/mod.rs:33
fn get_cached_text_width(text: &str, font_id: &FontId, color: Color32, ui: &Ui) -> f32 {
    let key = (text.to_string(), font_id.size as u32, color); // ALOCA STRING TODA VEZ
    FONT_WIDTH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(&width) = cache.get(&key) { return width; }
```

**Passos de implementação:**
1. Mudar o HashMap para usar `u64` como chave (hash pré-computado)
2. Usar `FxHasher` ou `hash_one()` para computar hash de `(text_bytes, font_size)` sem alocar:
```rust
use std::hash::{Hash, Hasher};
let mut hasher = rustc_hash::FxHasher::default();
text.hash(&mut hasher);
(font_id.size as u32).hash(&mut hasher);
let key = hasher.finish();
```
3. Nota: Color32 pode ser removido da chave — a largura de texto não depende de cor

**Arquivos afetados:**
- `src/ui/views/list_view/mod.rs` (função `get_cached_text_width`)

---

### A3. `contains_ignore_case` aloca dois `Vec<char>` por chamada

**Impacto: ALTO | Complexidade: MODERADA | Degrada UX: NÃO**

Na função de busca `contains_ignore_case` em `sorting.rs`, dois `Vec<char>` são alocados por comparação. Quando filtrando 10.000 itens, são 20.000 alocações de Vec.

**Evidência:**
```rust
// sorting.rs:131-132
let needle_lower: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
let haystack_chars: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
```

**Passos de implementação:**
1. Pré-computar `needle_lower` UMA VEZ em `filter_items_opt`, antes do loop:
```rust
pub fn filter_items_opt(items: &[FileEntry], query: &str) -> Option<Vec<FileEntry>> {
    if query.is_empty() { return None; }
    let needle_lower: Vec<u8> = query.bytes().map(|b| b.to_ascii_lowercase()).collect();
    Some(items.iter()
        .filter(|item| contains_ignore_case_precomputed(&item.name, &needle_lower))
        .cloned()
        .collect())
}
```
2. Para o haystack, usar comparação byte-a-byte para ASCII (maioria dos nomes de arquivo):
```rust
fn contains_ignore_case_precomputed(haystack: &str, needle_lower: &[u8]) -> bool {
    if haystack.is_ascii() && needle_lower.iter().all(|b| b.is_ascii()) {
        haystack.as_bytes().windows(needle_lower.len())
            .any(|w| w.iter().zip(needle_lower).all(|(h, n)| h.to_ascii_lowercase() == *n))
    } else {
        // Fallback Unicode para nomes com acentos
        let haystack_chars: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
        let needle_chars: Vec<char> = std::str::from_utf8(needle_lower)
            .unwrap_or("").chars().collect();
        haystack_chars.windows(needle_chars.len())
            .any(|w| w == needle_chars.as_slice())
    }
}
```

**Arquivos afetados:**
- `src/application/sorting.rs` (funções `contains_ignore_case` e `filter_items_opt`)

---

### A4. Passar RGBA data diretamente para GPU sem round-trip pelo cache

**Impacto: MÉDIO-ALTO | Complexidade: MODERADA | Degrada UX: NÃO**

Em `message_handler.rs`, o fluxo de upload de thumbnail faz:
1. `put_rgba_data(path, data, w, h)` — armazena no cache RAM
2. `get_rgba_data(&path)` — IMEDIATAMENTE recupera o que acabou de guardar
3. `load_texture(...)` — envia para GPU

O step 2 é redundante. A `data` original já está disponível antes de ser movida para o cache.

**Evidência:**
```rust
// message_handler.rs:1036-1060
self.cache_manager.put_rgba_data(path.clone(), thumbnail_data.image_data, width, height);
// ↓ Imediatamente busca o que acabou de guardar
let texture = if let Some((rgba_data, _, _)) = self.cache_manager.get_rgba_data(&path) {
    ctx.load_texture(...)
```

**Passos de implementação:**
1. Extrair `image_data` de `thumbnail_data` ANTES de mover para cache
2. Usar referência local para `load_texture`:
```rust
let rgba_data = thumbnail_data.image_data;
let texture = ctx.load_texture(
    path.to_string_lossy().to_string(),
    egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba_data),
    egui::TextureOptions::LINEAR,
);
// Depois, mover os dados para cache RAM (para re-upload futuro se evicted)
self.cache_manager.put_rgba_data(path.clone(), rgba_data, width, height);
```

**Arquivos afetados:**
- `src/app/operations/message_handler.rs` (bloco de upload, linhas 1036-1060)

---

### A5. `visible_paths` FxHashSet alocado a cada frame durante scroll

**Impacto: MÉDIO | Complexidade: MODERADA | Degrada UX: NÃO**

Durante scroll, `message_handler.rs` aloca um novo `FxHashSet<PathBuf>` CADA FRAME para priorizar uploads de thumbnails visíveis. Cada entrada requer clone de `PathBuf`.

**Evidência:**
```rust
// message_handler.rs:972-987
let visible_paths: Option<FxHashSet<PathBuf>> = if is_scrolling {
    self.visible_index_range.and_then(|(min_idx, max_idx)| {
        Some((min_idx..=max_idx).map(|i| items[i].path.clone()).collect())
    })
```

**Passos de implementação:**
1. Adicionar campo `visible_paths_cache: FxHashSet<PathBuf>` no `ImageViewerApp`
2. Adicionar campo `visible_range_cached: Option<(usize, usize)>` para detectar mudança
3. Apenas reconstruir se o range visível mudou:
```rust
if self.visible_range_cached != self.visible_index_range {
    self.visible_paths_cache.clear();
    if let Some((min, max)) = self.visible_index_range {
        let max = max.min(items.len().saturating_sub(1));
        for i in min..=max {
            self.visible_paths_cache.insert(items[i].path.clone());
        }
    }
    self.visible_range_cached = self.visible_index_range;
}
let visible_paths = if is_scrolling { Some(&self.visible_paths_cache) } else { None };
```

**Arquivos afetados:**
- `src/app/operations/message_handler.rs`
- `src/app/state.rs` (adicionar campos)

---

## TIER B — IMPACTO MÉDIO, COMPLEXIDADE SIMPLES

*Ganhos menores mas implementação rápida e segura.*

---

### B1. TextureOptions::LINEAR para thumbnails em vez de NEAREST

**Impacto: MÉDIO (melhora visual) | Complexidade: SIMPLES | Degrada UX: NÃO — MELHORA**

Thumbnails usam `TextureOptions::NEAREST` que causa pixelação visível quando o thumbnail é exibido em tamanho diferente do gerado (ex: zoom slider entre buckets de 128/256/512). Icons já usam `LINEAR` e ficam suaves. Mudar para `LINEAR` melhora a qualidade visual sem custo mensurável de GPU — filtragem bilinear é praticamente gratuita em hardware moderno.

**Evidência:**
- [message_handler.rs:809](src/app/operations/message_handler.rs#L809): `TextureOptions::NEAREST` (icons async)
- [message_handler.rs:1055](src/app/operations/message_handler.rs#L1055): `TextureOptions::NEAREST` (thumbnails)
- [message_handler.rs:1102](src/app/operations/message_handler.rs#L1102): `TextureOptions::NEAREST` (folder previews)

**Passos de implementação:**
1. Em `message_handler.rs` linhas 809, 1055, 1102: mudar `NEAREST` para `LINEAR`
2. Testar visualmente com thumbnails em zoom variado (slider 64-256)

**Arquivos afetados:**
- `src/app/operations/message_handler.rs` (3 ocorrências)

---

### B2. `idle_visible_items` Vec alocado a cada frame

**Impacto: MÉDIO | Complexidade: SIMPLES | Degrada UX: NÃO**

Tanto grid_view quanto list_view alocam `let mut idle_visible_items = Vec::new()` a cada frame e preenchem com clones de `PathBuf` dos itens visíveis.

**Evidência:**
```rust
// grid_view.rs:781-789
let mut idle_visible_items = Vec::new();
for index in first_visible_index..=last_visible_index {
    let item = &ctx.items[index];
    if !item.is_dir {
        idle_visible_items.push(item.path.clone()); // CLONE POR ITEM POR FRAME
    }
}
```
Mesmo padrão em [list_view/virtualization.rs:386-394](src/ui/views/list_view/virtualization.rs#L386).

**Passos de implementação:**
1. Mover `idle_visible_items` para `GridViewContext` e `ListViewContext` como buffer reutilizável
2. Chamar `.clear()` em vez de criar novo Vec a cada frame
3. Usar `Vec::with_capacity(expected_visible_count)` na primeira alocação

**Arquivos afetados:**
- `src/ui/views/grid_view.rs` (linha 781)
- `src/ui/views/list_view/virtualization.rs` (linha 386)

---

### B3. Cache de evicção da font_width_cache: LRU em vez de clear total

**Impacto: MÉDIO | Complexidade: SIMPLES | Degrada UX: NÃO**

O cache de largura de fontes limpa TUDO quando atinge 5000 entradas. Isso causa spike de medições na frame seguinte quando todo o cache é reconstruído.

**Evidência:**
```rust
// list_view/mod.rs:43-44
if cache.len() > 5000 {
    cache.clear(); // CLEAR TOTAL — perde TUDO de uma vez
}
```

**Passos de implementação:**
1. Substituir `HashMap` por `LruCache` (já é dependência: `lru = "0.12"`)
2. Criar com capacidade de 5000 entradas: `LruCache::new(NonZeroUsize::new(5000).unwrap())`
3. Remover a verificação manual de tamanho — LRU faz evicção automática dos menos usados

**Arquivos afetados:**
- `src/ui/views/list_view/mod.rs` (linhas 20-50)

---

### B4. Scroll smoothing threshold muito baixo

**Impacto: MÉDIO | Complexidade: SIMPLES | Degrada UX: NÃO**

O smooth scroll no grid view força repaint a 60fps (`request_repaint_after(16ms)`) mesmo quando a diferença entre visual e target é sub-pixel. A comparação `visual_scroll != scroll_target` usa igualdade de float que pode nunca ser exatamente verdadeira.

**Evidência:**
```rust
// grid_view.rs:432-443
if (state.visual_scroll_y - scroll_target).abs() < 1.0 {
    state.visual_scroll_y = scroll_target; // Snap quando < 1px
}
// ...
if visual_scroll != scroll_target {  // Float comparison — pode ser infinito
    ui.ctx().request_repaint_after(Duration::from_millis(16));
}
```

**Passos de implementação:**
1. Mudar a condição de repaint para threshold explícito:
```rust
if (visual_scroll - scroll_target).abs() > 0.5 {
    ui.ctx().request_repaint_after(Duration::from_millis(16));
}
```
2. Isso evita repaints infinitos por imprecisão de ponto flutuante

**Arquivos afetados:**
- `src/ui/views/grid_view.rs` (linhas 441-443)

---

### B5. Overscan adaptativo baseado em velocidade de scroll no grid view

**Impacto: MÉDIO | Complexidade: SIMPLES | Degrada UX: NÃO — MELHORA**

O grid view usa overscan fixo de 2 linhas. Durante scroll rápido, áreas brancas podem aparecer momentaneamente. O list view já tem overscan adaptativo (2 durante scroll, 5 quando idle).

**Evidência:**
```rust
// grid_view.rs:522
let overscan = 2; // FIXO — sem adaptação

// vs list_view/virtualization.rs:235
let overscan = if is_scrolling { 2 } else { 5 }; // ADAPTATIVO
```

O grid view já tem `ScrollPredictor` com campo `velocity` que não é usado para ajustar overscan.

**Passos de implementação:**
1. Usar `ScrollPredictor.velocity` para escalar overscan:
```rust
let overscan = if is_scrolling {
    if ctx.scroll_predictor.velocity > 5.0 { 3 } else { 2 }
} else { 4 };
```

**Arquivos afetados:**
- `src/ui/views/grid_view.rs` (linha 522)

---

## TIER C — IMPACTO MÉDIO, COMPLEXIDADE MODERADA/COMPLEXA

*Melhorias significativas mas exigem mais refatoração.*

---

### C1. `filter_items_opt` clona FileEntry inteiros

**Impacto: MÉDIO | Complexidade: MODERADA | Degrada UX: NÃO**

Quando há busca ativa, `filter_items_opt` clona cada `FileEntry` que bate no filtro. `FileEntry` contém `PathBuf`, `String`, `Option<PathBuf>`, `Option<String>`, `Option<PathBuf>` — 5 campos heap-allocated.

**Evidência:**
```rust
// sorting.rs:149-153
Some(items.iter()
    .filter(|item| contains_ignore_case(&item.name, query))
    .cloned()  // CLONA FileEntry COMPLETO
    .collect())
```

**Passos de implementação:**
1. Mudar para retornar índices em vez de items clonados:
```rust
pub fn filter_indices(items: &[FileEntry], query: &str) -> Option<Vec<usize>> {
    if query.is_empty() { return None; }
    Some(items.iter().enumerate()
        .filter(|(_, item)| contains_ignore_case(&item.name, query))
        .map(|(i, _)| i)
        .collect())
}
```
2. Na thread de rebuild, usar índices para construir o Vec final uma vez

**Nota:** A busca já roda em `std::thread::spawn` (não no UI thread). O impacto real é em memória, não em latência de frame. Prioridade menor que Tier A.

**Arquivos afetados:**
- `src/application/sorting.rs`
- `src/app/operations/message_handler.rs` (spawn de rebuild)

---

### C2. Smooth scroll para list view

**Impacto: MÉDIO (UX) | Complexidade: MODERADA | Degrada UX: NÃO — MELHORA**

O grid view tem smooth scroll com spring interpolation, mas o list view faz scroll direto. A diferença é perceptível ao alternar entre os dois modos de visualização.

**Evidência:**
```rust
// list_view/virtualization.rs:38-45
let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
if scroll_delta != 0.0 {
    *ctx.mut_scroll_offset_y -= scroll_delta * 5.0;
}
// SEM INTERPOLAÇÃO — snap direto
```

vs grid_view.rs com spring physics (fator 25.0).

**Passos de implementação:**
1. Adicionar `target_scroll_y` e `visual_scroll_y` ao `ListViewContext` (mesmo padrão do grid)
2. Copiar a lógica de interpolação spring do grid_view (linhas 413-438)
3. Usar `visual_scroll_y` para rendering, `target_scroll_y` para input
4. Adicionar `request_repaint_after(16ms)` durante animação

**Arquivos afetados:**
- `src/ui/views/list_view/virtualization.rs`
- `src/ui/views/list_view/mod.rs` (adicionar campos ao `ListViewContext`)

---

### C3. Reduzir chamadas `ui.interact()` e `ui.put()` no item_slot

**Impacto: MÉDIO | Complexidade: COMPLEXA | Degrada UX: NÃO**

Cada grid item chama `ui.interact()` (grid_view.rs:245), e item_slot.rs faz até 8 chamadas `ui.put()` por item. Para 100 itens visíveis, são 100+ interact + potencialmente 800 put por frame.

**Onde já usa `painter.image()` (mais barato):**
- [item_slot.rs:296](src/ui/components/item_slot.rs#L296): folder preview — `painter.image()` direto
- [item_slot.rs:539](src/ui/components/item_slot.rs#L539): thumbnail — `painter.image()` direto

**Onde usa `ui.put()` (mais caro, inclui layout):**
- [item_slot.rs:125](src/ui/components/item_slot.rs#L125): drive icon — `ui.put(icon_rect, Image::new(...))`
- [item_slot.rs:181](src/ui/components/item_slot.rs#L181): drive name label
- [item_slot.rs:321](src/ui/components/item_slot.rs#L321), [338](src/ui/components/item_slot.rs#L338): folder icons

**Passos de implementação (progressivo):**
1. **Rápido:** Converter `ui.put(rect, Image::new(...))` para `ui.painter().image(tex.id(), rect, ...)` onde o rect já está calculado (ícones de tamanho fixo)
2. **Médio:** Cachear IDs por item no loop (calcular `ui.id().with(index)` uma vez)
3. **Complexo:** Substituir interação por-item por hit-testing baseado em área (calcular qual item o mouse está sobre usando posição do mouse + grid layout, em vez de registrar interação em cada item)

**Arquivos afetados:**
- `src/ui/components/item_slot.rs`
- `src/ui/views/grid_view.rs`

---

### C4. Cache de `normalize_for_match` no message_handler

**Impacto: BAIXO-MÉDIO | Complexidade: SIMPLES | Degrada UX: NÃO**

`normalize_for_match` aparece **32 vezes** em `message_handler.rs`. Muitas chamadas são para o MESMO `self.current_path` dentro do mesmo frame.

**Evidência:** `current_path_norm` já é computado uma vez na linha 403 para watcher events. Mas dentro dos loops de `file_op_res_receiver`, `normalize_for_match(Path::new(&self.current_path))` é recomputado para cada resultado.

**Passos de implementação:**
1. Computar `current_path_norm` UMA VEZ no início de `process_incoming_messages`
2. Passar como referência para todos os blocos internos

**Arquivos afetados:**
- `src/app/operations/message_handler.rs`

---

## MELHORIAS ADICIONAIS

---

### #29. Cache de `is_media_extension` como campo bool em FileEntry

**Impacto: MÉDIO-ALTO | Complexidade: MODERADA | Degrada UX: NÃO**

Em `item_slot.rs:461-464` e `item_renderer.rs:40-43`, `is_media_extension()` faz lookup na Windows Registry (via `get_perceived_type()`) POR ARQUIVO POR FRAME para determinar se deve carregar thumbnail.

**Evidência:**
```rust
// item_slot.rs:461-464
let is_media_file = path_clone
    .extension()
    .map(|ext| crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy()))
    .unwrap_or(false);
```

**Passos de implementação:**
1. Adicionar campo `is_media: bool` em `FileEntry`
2. Calcular durante enumeração do diretório (uma vez por arquivo)
3. Substituir chamadas runtime por `item.is_media`

**Arquivos afetados:**
- `src/domain/file_entry.rs`
- `src/infrastructure/windows/file_type.rs`
- `src/app/operations/folder_loading.rs`
- `src/ui/components/item_slot.rs`
- `src/ui/views/list_view/item_renderer.rs`

---

### #30. Extrair tooltip com debounce para função compartilhada

**Impacto: BAIXO (manutenção) | Complexidade: SIMPLES | Degrada UX: NÃO**

O código de tooltip com debounce está duplicado quase idêntico em:
1. [grid_view.rs:289-377](src/ui/views/grid_view.rs#L289) (grid items)
2. [item_renderer.rs:208-293](src/ui/views/list_view/item_renderer.rs#L208) (list items)

Extrair para função compartilhada reduz manutenção e tamanho de código.

---

### D1. `get_file_type_string` duplicado em 3+ módulos

**Impacto: BAIXO | Complexidade: SIMPLES | Degrada UX: NÃO**

Existem implementações separadas:
- `src/ui/views/common.rs` — aloca `String` sempre
- `src/ui/views/list_view/helpers.rs` — retorna `Cow<str>` (otimizado)
- `src/ui/views/grid_view.rs:925` — retorna `Cow<str>` (otimizado)

Manter apenas a versão com `Cow<str>` e centralizar.

---

### D2. `ui.put()` vs `painter.image()` para ícones de tamanho fixo

**Impacto: BAIXO-MÉDIO | Complexidade: SIMPLES | Degrada UX: NÃO**

Ícones com tamanho fixo que não precisam de layout podem usar `painter.image()` direto, que é mais leve que `ui.put(rect, Image::new(...))`.

---

### D3. Pré-alocar `pending_thumbnails` VecDeque

**Impacto: BAIXO | Complexidade: SIMPLES | Degrada UX: NÃO**

`pending_thumbnails` é um `VecDeque` que cresce dinamicamente. Pré-alocar com `VecDeque::with_capacity(100)` evita realocações.

---

### D4. Evitar `FontFamily::Name("icons".into())` repetido

**Impacto: BAIXO | Complexidade: SIMPLES | Degrada UX: NÃO**

Em `item_renderer.rs`, `FontFamily::Name("icons".into())` aparece em múltiplos lugares, alocando um `Arc<str>` a cada chamada. Poderia ser uma constante `thread_local!` ou `lazy_static`.

---

## ITENS DESCARTADOS (após análise)

| Item | Motivo |
|------|--------|
| `ui.interact(viewport_rect)` para background click | Necessário para detectar right-click em área vazia. Uma chamada por frame, não por item. |
| `keep_paths FxHashSet` no loading_set cleanup | Já usa `FxHashSet<&PathBuf>` com referências, sem clones. Já otimizado. |
| Deferred thumbnail re-queuing | `deferred_count` limiter já impede loops. Melhoria seria marginal. |
| Cache de egui ID `ui.id().with(index)` | Operação de hash puro sem alocação. Custo negligível. |
| `Arc<Vec<FileEntry>>` clone | `Arc::clone` é O(1) — incremento atômico. Já ótimo. |

---

## TABELA RESUMO

| # | Item | Impacto | Complexidade | Degrada UX? |
|---|------|---------|-------------|-------------|
| **S1** | Remover eprintln! em release | ALTO | SIMPLES | NÃO |
| **S2** | Eliminar to_lowercase().ends_with(".zip") | ALTO | SIMPLES | NÃO |
| **S3** | Eliminar path.clone() em render_file_slot | ALTO | SIMPLES | NÃO |
| **A1** | Cache de truncação de texto | ALTO | MODERADA | NÃO |
| **A2** | Chave do font cache sem String | ALTO | MODERADA | NÃO |
| **A3** | contains_ignore_case sem Vec<char> | ALTO | MODERADA | NÃO |
| **A4** | RGBA direto para GPU | MÉDIO-ALTO | MODERADA | NÃO |
| **A5** | visible_paths cache persistente | MÉDIO | MODERADA | NÃO |
| **B1** | TextureOptions::LINEAR | MÉDIO | SIMPLES | **MELHORA** |
| **B2** | idle_visible_items buffer reutilizável | MÉDIO | SIMPLES | NÃO |
| **B3** | Font cache LRU | MÉDIO | SIMPLES | NÃO |
| **B4** | Scroll threshold > 0.5px | MÉDIO | SIMPLES | NÃO |
| **B5** | Overscan adaptativo grid | MÉDIO | SIMPLES | **MELHORA** |
| **C1** | filter_items por índices | MÉDIO | MODERADA | NÃO |
| **C2** | Smooth scroll list view | MÉDIO | MODERADA | **MELHORA** |
| **C3** | Reduzir ui.interact/put | MÉDIO | COMPLEXA | NÃO |
| **C4** | Cache normalize_for_match | BAIXO-MÉDIO | SIMPLES | NÃO |
| **#29** | Cache is_media_extension | MÉDIO-ALTO | MODERADA | NÃO |
| **#30** | Tooltip compartilhado | BAIXO | SIMPLES | NÃO |
| **D1** | Consolidar get_file_type_string | BAIXO | SIMPLES | NÃO |
| **D2** | painter.image() para ícones fixos | BAIXO-MÉDIO | SIMPLES | NÃO |
| **D3** | Pré-alocar VecDeque | BAIXO | SIMPLES | NÃO |
| **D4** | FontFamily::Name constante | BAIXO | SIMPLES | NÃO |

---

## Arquivos Críticos para Implementação

| Arquivo | Otimizações Relacionadas |
|---------|------------------------|
| `src/app/operations/message_handler.rs` | S1, A4, A5, B1, C4 |
| `src/ui/components/item_slot.rs` | S2, S3, #29, C3, D2 |
| `src/ui/views/list_view/mod.rs` | A1, A2, B3 |
| `src/ui/views/grid_view.rs` | S2, B2, B4, B5, #30 |
| `src/application/sorting.rs` | A3, C1 |
| `src/domain/file_entry.rs` | S2, #29, D1 |
| `src/ui/views/list_view/item_renderer.rs` | S2, #29, #30, D4 |
| `src/ui/views/list_view/virtualization.rs` | B2, C2 |
| `src/app/state.rs` | A5 |
