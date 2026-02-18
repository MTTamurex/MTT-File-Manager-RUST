# Plano de Implementação: Volume de Sessão + Trava de Pasta (Cadeado)

## Contexto

Duas mudanças solicitadas:
1. **Volume**: Atualmente `app.saved_media_volume` é carregado do SQLite no startup e nunca atualizado em runtime. Quando um novo vídeo abre, sempre usa o valor do disco, ignorando ajustes do usuário na sessão. Precisa de um campo `session_volume` que acompanhe mudanças do slider/teclado e seja salvo no disco apenas ao sair.
2. **Trava de Pasta**: Novo botão cadeado que congela preferências de exibição (view_mode, sort_mode, sort_descending, folders_position, search_query) para uma pasta específica, persistido via SQLite.

---

## TASK 1: Correção da Lógica de Volume

### 1.1 Renomear campo no state
**Arquivo:** `src/app/state.rs` (linha 294)
- Renomear `saved_media_volume` → `session_volume`

### 1.2 Renomear no StartupPreferences
**Arquivo:** `src/app/init_preferences.rs`
- Linha 20: `pub(super) saved_media_volume` → `pub(super) session_volume`
- Linhas 130-134: variável local `saved_media_volume` → `session_volume`
- Struct initializer: campo correspondente

### 1.3 Atualizar init.rs
**Arquivo:** `src/app/init.rs`
- Linha 133 (destructure): `saved_media_volume` → `session_volume`
- Linha 377 (struct init): `saved_media_volume` → `session_volume`

### 1.4 Usar session_volume ao abrir vídeo
**Arquivo:** `src/ui/app/panels.rs` (linha 225)
- `player.initial_volume = app.saved_media_volume` → `player.initial_volume = app.session_volume`

### 1.5 Propagar mudança de volume do slider → app state
**Arquivo:** `src/ui/preview_panel/actions.rs`
- Adicionar variante: `VolumeChanged(f32)` ao enum `PreviewPanelAction`

**Arquivo:** `src/ui/preview_panel/video_preview/controls.rs`
- `draw_basic_controls`: retornar `Option<f32>` (novo volume quando slider muda)
- `draw_docked_controls`: retornar `Option<f32>`, propagar de `draw_basic_controls`
- `draw_detached_controls`: retornar `Option<f32>`, propagar de `draw_basic_controls`
- `draw_video_controls`: retornar `Option<f32>`, propagar

**Arquivo:** `src/ui/preview_panel/video_preview/docked.rs`
- `render_docked_video`: retornar `Option<PreviewPanelAction>`, converter `Some(vol)` → `PreviewPanelAction::VolumeChanged(vol)`

**Arquivo:** `src/ui/preview_panel/video_preview/detached.rs`
- `render_detached_video`: retornar `Option<PreviewPanelAction>`, converter igual

**Arquivo:** `src/ui/preview_panel/video_preview/fullscreen.rs`
- `render_fullscreen_video`: retornar `Option<PreviewPanelAction>`, converter igual

**Arquivo:** `src/ui/preview_panel/video_preview/mod.rs`
- Em `render_video_preview`: capturar retorno de render_docked/detached/fullscreen e combinar com `action` existente (usar `action = action.or(volume_action)`)

**Arquivo:** `src/ui/app/panels.rs` (match do PreviewPanelAction)
- Adicionar arm: `PreviewPanelAction::VolumeChanged(vol) => { app.session_volume = vol; }`

### 1.6 Propagar mudança de volume do teclado → app state
**Arquivo:** `src/ui/app/input.rs` (linhas 287-300)
- Na função `handle_media_hardware_input`: capturar `new_vol` em variável local `new_session_vol: Option<f32>`
- Nos blocos VK_UP (linha 289) e VK_DOWN (linha 296): `new_session_vol = Some(new_vol)`
- Após o bloco `unsafe` (quando o borrow de `preview` já foi liberado): `if let Some(v) = new_session_vol { app.session_volume = v; }`
- **Nota**: `preview` é `&mut app.media_preview` portanto `app.session_volume` não pode ser escrito dentro do mesmo bloco. A solução é usar uma variável local e escrever depois.

### 1.7 Salvar session_volume em vez de ler do player
**Arquivo:** `src/app/operations/preferences.rs` (linhas 145-150)
- Substituir bloco condicional que lê do player ativo por:
  ```rust
  prefs.push(("media_volume", self.session_volume.to_string()));
  ```

---

## TASK 2: Trava de Visualização por Pasta (Botão Cadeado)

### 2.1 Struct FolderLock
**Novo arquivo:** `src/domain/folder_lock.rs`
```rust
pub struct FolderLock {
    pub view_mode: ViewMode,
    pub sort_mode: SortMode,
    pub sort_descending: bool,
    pub folders_position: FoldersPosition,
    pub search_query: String,
}
```
**Arquivo:** `src/domain/mod.rs` — adicionar `pub mod folder_lock; pub use folder_lock::FolderLock;`

### 2.2 Tabela SQLite
**Arquivo:** `src/infrastructure/disk_cache.rs` — em `run_migrations()` (após linha ~258)
- `CREATE TABLE IF NOT EXISTS folder_locks (path TEXT PRIMARY KEY, view_mode TEXT, sort_mode TEXT, sort_descending TEXT, folders_position TEXT, search_query TEXT)`

### 2.3 Operações SQLite
**Novo arquivo:** `src/infrastructure/disk_cache/folder_locks.rs`
- `save_folder_lock(&self, path: &str, lock: &FolderLock)` — INSERT OR REPLACE (writer)
- `remove_folder_lock(&self, path: &str)` — DELETE (writer)
- `get_all_folder_locks(&self) -> HashMap<String, FolderLock>` — SELECT * (reader)
- Usar mesmos padrões de serialização string que `preferences.rs` (grid/list, name/date/size/type, true/false, first/last/mixed)

**Arquivo:** `src/infrastructure/disk_cache.rs` — adicionar `mod folder_locks;`

### 2.4 Campos no App State
**Arquivo:** `src/app/state.rs` (próximo a linha 294)
```rust
pub folder_locks: HashMap<String, FolderLock>,
pub current_folder_locked: bool,
```

### 2.5 Carregar folder_locks no startup
**Arquivo:** `src/app/init.rs`
- Após carregar preferences: `let folder_locks = disk_cache.get_all_folder_locks();`
- No bloco `Self { .. }`: `folder_locks, current_folder_locked: false,`

### 2.6 Lógica toggle + apply
**Novo arquivo:** `src/app/operations/folder_lock_ops.rs`

`toggle_folder_lock(&mut self)`:
- Se `current_folder_locked`: remove do HashMap e SQLite, seta false
- Se não: captura view_mode/sort_mode/sort_descending/folders_position/search_query atuais, salva no HashMap e SQLite, seta true
- Ignora vistas especiais (computer_view, recycle_bin, path vazio)

`apply_folder_lock_if_present(&mut self)`:
- Verifica se `current_path` está no HashMap
- Se sim: aplica todos os campos do FolderLock (view_mode, sort_mode, sort_descending, folders_position, search_query), seta `current_folder_locked = true`
- Se não: seta `current_folder_locked = false`

**Arquivo:** `src/app/operations/mod.rs` — adicionar `pub mod folder_lock_ops;`

### 2.7 Integração com navegação
**Arquivo:** `src/app/operations/navigation/mod.rs`

Em `navigate_to()` — inserir `self.apply_folder_lock_if_present()` **após** `reset_selection_and_search()` (linha 98) e **antes** de `watch_current_folder()` (linha 101). Isso garante que:
- `sort_mode = sort_mode_normal` já executou (linha 93)
- `reset_selection_and_search` já limpou search_query (linha 98/159)
- O lock restaura search_query e sort_mode corretos

Em `go_back()` — no else branch (linhas 128-150): inserir `self.apply_folder_lock_if_present()` após `self.sort_mode = self.sort_mode_normal` (linha 146) e `reset_selection_and_search` (linha 148), antes de `load_folder` (linha 150)

Em `go_forward()` — no else branch (linhas 178-200): mesmo padrão, após linhas 196-198, antes de `load_folder` (linha 200)

### 2.8 Ícones SVG
**Novos arquivos:** `assets/icons/lock.svg` e `assets/icons/lock_open.svg`
- Ícones de cadeado simples, estilo Lucide/Feather (consistente com os outros ícones do projeto)

**Arquivo:** `src/embedded_assets.rs`
- Adicionar `const ICON_LOCK` e `ICON_LOCK_OPEN` com `include_bytes!`
- Adicionar match arms: `"lock" => Some(ICON_LOCK)`, `"lock_open" => Some(ICON_LOCK_OPEN)`

### 2.9 Botão cadeado na toolbar secundária
**Arquivo:** `src/ui/app/layers/secondary_toolbar_layer.rs`

Após `sort_controls::render_sort_controls(ui, app)` (linha 51):
- Adicionar separador + chamada a função `render_lock_button`
- `render_lock_button` renderiza o ícone lock/lock_open usando `svg_manager.get_icon`
- Usar `widgets::toggle_icon_button` ou custom render seguindo o padrão existente
- Se clicado: chamar `app.toggle_folder_lock()` e se travou, `app.filter_items()` + `app.sort_items()`
- Desabilitar o botão em vistas especiais (computer_view, recycle_bin)
- Atualizar `content_width` (linha 42) para incluir o novo botão (~36px extra)

### 2.10 Desabilitar controles quando travado
**Arquivo:** `src/ui/app/layers/secondary_toolbar_layer/sort_controls.rs`
- Wrapping com `ui.scope(|ui| { if app.current_folder_locked { ui.set_enabled(false); } ... })`

**Arquivo:** `src/ui/app/layers/secondary_toolbar_layer/view_zoom_controls.rs`
- Mesmo padrão: `ui.set_enabled(false)` quando `current_folder_locked`

**Arquivo:** `src/ui/status_bar.rs`
- Adicionar parâmetro `folder_locked: bool` em `render_status_bar`
- Wrapping das seções de view mode, sort, folders position com `ui.set_enabled(false)` quando locked

**Arquivo:** `src/ui/app/layers/status_bar_layer.rs`
- Passar `app.current_folder_locked` na chamada a `render_status_bar`

**Arquivo:** `src/ui/toolbar.rs` (search box)
- Quando `current_folder_locked`: search box com `ui.set_enabled(false)` ou `.interactive(false)`

---

## Arquivos Afetados (Resumo)

### Task 1 (8 arquivos):
| Arquivo | Tipo |
|---|---|
| `src/app/state.rs` | Editar |
| `src/app/init_preferences.rs` | Editar |
| `src/app/init.rs` | Editar |
| `src/app/operations/preferences.rs` | Editar |
| `src/ui/app/panels.rs` | Editar |
| `src/ui/app/input.rs` | Editar |
| `src/ui/preview_panel/actions.rs` | Editar |
| `src/ui/preview_panel/video_preview/controls.rs` | Editar |
| `src/ui/preview_panel/video_preview/docked.rs` | Editar |
| `src/ui/preview_panel/video_preview/detached.rs` | Editar |
| `src/ui/preview_panel/video_preview/fullscreen.rs` | Editar |
| `src/ui/preview_panel/video_preview/mod.rs` | Editar |

### Task 2 (14 arquivos, 3 novos):
| Arquivo | Tipo |
|---|---|
| `src/domain/folder_lock.rs` | **Novo** |
| `src/domain/mod.rs` | Editar |
| `src/infrastructure/disk_cache.rs` | Editar |
| `src/infrastructure/disk_cache/folder_locks.rs` | **Novo** |
| `src/app/state.rs` | Editar |
| `src/app/init.rs` | Editar |
| `src/app/operations/mod.rs` | Editar |
| `src/app/operations/folder_lock_ops.rs` | **Novo** |
| `src/app/operations/navigation/mod.rs` | Editar |
| `src/ui/app/layers/secondary_toolbar_layer.rs` | Editar |
| `src/ui/app/layers/secondary_toolbar_layer/sort_controls.rs` | Editar |
| `src/ui/app/layers/secondary_toolbar_layer/view_zoom_controls.rs` | Editar |
| `src/ui/status_bar.rs` | Editar |
| `src/ui/app/layers/status_bar_layer.rs` | Editar |
| `src/ui/toolbar.rs` | Editar |
| `src/embedded_assets.rs` | Editar |
| `assets/icons/lock.svg` | **Novo** |
| `assets/icons/lock_open.svg` | **Novo** |

---

## Verificação

### Task 1 — Volume:
1. `cargo build` — compilar sem erros
2. Abrir app, tocar um vídeo, ajustar volume pelo slider
3. Abrir outro vídeo → deve manter o volume ajustado (não resetar ao valor do disco)
4. Ajustar volume pelo teclado (↑/↓) → abrir outro vídeo → deve manter
5. Fechar e reabrir o app → volume deve ser o último valor da sessão anterior

### Task 2 — Trava de Pasta:
1. `cargo build` — compilar sem erros
2. Navegar até uma pasta, configurar view mode/sort/filters desejados
3. Clicar no cadeado → ícone deve mudar para "trancado"
4. Controles de sort/view/search devem ficar desabilitados (cinza)
5. Navegar para outra pasta → controles devem voltar ao normal
6. Voltar para a pasta travada → preferências restauradas, controles travados
7. Clicar no cadeado para destravar → controles liberados
8. Fechar e reabrir o app → pasta deve continuar travada com as preferências salvas
