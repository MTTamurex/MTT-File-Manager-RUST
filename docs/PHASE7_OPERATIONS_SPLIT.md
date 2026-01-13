# Phase 7: Split `app/operations.rs` (2461 lines → ~300 lines each)

**Data**: 13/01/2026  
**Status**: 📋 PLANEJADO  
**Prioridade**: ALTA  
**Estimativa**: 2-3 horas

---

## 📋 Objetivo

Dividir o arquivo monolítico `src/app/operations.rs` (2461 linhas, 61 funções) em módulos menores e coesos, seguindo o princípio de responsabilidade única.

---

## 📊 Análise do Estado Atual

### Mapeamento de Funções por Responsabilidade

| Linha | Função | Categoria Proposta |
|-------|--------|-------------------|
| 47 | `delete_with_shell_for_idx` | file_ops |
| 66 | `restore_from_recycle_bin` | recycle_bin_ops |
| 109 | `delete_permanently` | recycle_bin_ops |
| 136 | `empty_recycle_bin` | recycle_bin_ops |
| 158 | `show_properties_for_idx` | file_ops |
| 172 | `create_new_folder` | file_ops |
| 211 | `command_copy` | clipboard_ops |
| 220 | `command_cut` | clipboard_ops |
| 230 | `command_paste` | clipboard_ops |
| 267 | `filter_items` | items |
| 275 | `sort_items` | items |
| 286 | `save_preferences` | preferences |
| 356 | `request_folder_scan` | folder_loading |
| 361 | `load_folder` | folder_loading |
| 560 | `navigate_to` | navigation |
| 602 | `go_back` | navigation |
| 643 | `go_forward` | navigation |
| 682 | `navigate_to_computer` | navigation |
| 701 | `navigate_to_recycle_bin` | navigation |
| 718 | `setup_recycle_bin_view` | view_setup |
| 814 | `setup_computer_view` | view_setup |
| 850 | `reload_drive_list` | view_setup |
| 859 | `refresh_drives_if_needed` | view_setup |
| 868 | `trigger_manual_refresh` | folder_loading |
| 879 | `sync_to_tab` | tabs |
| 904 | `sync_from_tab` | tabs |
| 922 | `go_up_one_level` | navigation |
| 939 | `watch_current_folder` | watcher |
| 976 | `rename_with_shell` | file_ops |
| 1017 | `can_go_back` | navigation |
| 1022 | `can_go_forward` | navigation |
| 1026 | `request_thumbnail_load` | thumbnails |
| 1031 | `request_folder_preview_load` | thumbnails |
| 1041 | `ensure_window_handle` | window |
| 1080 | `get_or_load_icon` | icons (DEAD CODE) |
| 1140 | `ensure_folder_icon` | icons |
| 1153 | `ensure_computer_icon` | icons |
| 1160 | `refresh_selected_metadata` | metadata |
| 1198 | `format_media_duration` | metadata (helper) |
| 1212 | `format_bitrate` | metadata (helper) |
| 1223 | `approximate_bitrate` | metadata (helper) |
| 1236 | `process_incoming_messages` | message_handler |
| 1543 | `render_list_view` | **MOVE TO ui/** |
| 1779 | `render_grid_view` | **MOVE TO ui/** |
| 1985 | `render_item_slot` | **MOVE TO ui/** |
| 2103-2146 | Duplicated closures | **DELETE** |
| 2154 | `update_selected_thumbnail` | selection |
| 2177 | `reset_selection_and_search` | selection |
| 2188 | `context_target_path` | context_menu |
| 2205 | `copy_path_to_clipboard` | clipboard_ops |
| 2212 | `create_shell_shortcut` | file_ops |
| 2216 | `populate_context_menu` | context_menu |

---

## 🏗️ Nova Estrutura Proposta

```
src/app/
├── mod.rs                    # Re-exports públicos
├── state.rs                  # ImageViewerApp struct (MANTER)
├── init.rs                   # fn new() (MANTER)
├── operations/
│   ├── mod.rs               # Módulo de operações
│   ├── file_ops.rs          # ~150 linhas
│   ├── clipboard_ops.rs     # ~80 linhas
│   ├── navigation.rs        # ~180 linhas
│   ├── folder_loading.rs    # ~250 linhas
│   ├── view_setup.rs        # ~150 linhas
│   ├── recycle_bin_ops.rs   # ~100 linhas
│   ├── tabs.rs              # ~50 linhas
│   ├── watcher.rs           # ~50 linhas
│   ├── preferences.rs       # ~80 linhas
│   ├── thumbnails.rs        # ~30 linhas
│   ├── icons.rs             # ~50 linhas
│   ├── metadata.rs          # ~100 linhas
│   ├── selection.rs         # ~50 linhas
│   ├── context_menu.rs      # ~200 linhas
│   ├── window.rs            # ~50 linhas
│   └── message_handler.rs   # ~350 linhas (MOVER de app/)
```

**Total estimado**: ~1900 linhas distribuídas em 16 arquivos (~120 linhas/arquivo)

---

## 📝 Plano de Execução Detalhado

### Passo 0: Preparação (ANTES de começar)

```bash
# Verificar baseline
cargo check
cargo test

# Criar backup (opcional)
cp src/app/operations.rs src/app/operations.rs.backup
```

### Passo 1: Criar estrutura de diretórios

```bash
mkdir src/app/operations
```

### Passo 2: Criar `src/app/operations/mod.rs`

```rust
//! Application operations split into focused modules.
//!
//! Each module handles a specific area of functionality:
//! - `file_ops`: File deletion, creation, renaming
//! - `clipboard_ops`: Copy, cut, paste operations
//! - `navigation`: Path navigation and history
//! - `folder_loading`: Async folder scanning and filtering
//! - `view_setup`: Computer view, recycle bin view setup
//! - `recycle_bin_ops`: Recycle bin specific operations
//! - `tabs`: Tab synchronization
//! - `watcher`: File system watcher management
//! - `preferences`: Save/load user preferences
//! - `thumbnails`: Thumbnail loading requests
//! - `icons`: Icon loading and caching
//! - `metadata`: Media metadata handling
//! - `selection`: Selection state management
//! - `context_menu`: Context menu population
//! - `window`: Window handle management
//! - `message_handler`: Async message processing

mod clipboard_ops;
mod context_menu;
mod file_ops;
mod folder_loading;
mod icons;
mod message_handler;
mod metadata;
mod navigation;
mod preferences;
mod recycle_bin_ops;
mod selection;
mod tabs;
mod thumbnails;
mod view_setup;
mod watcher;
mod window;

// Re-export nothing - all methods are impl on ImageViewerApp
```

### Passo 3: Extrair módulos (ordem de dependência)

#### 3.1 `file_ops.rs` (Linhas 47-65, 158-210, 976-1016, 2212-2215)

```rust
// src/app/operations/file_ops.rs
//! File operations: delete, create folder, rename, properties, shortcuts

use std::path::Path;
use crate::app::state::ImageViewerApp;
use crate::application::file_operations;

impl ImageViewerApp {
    pub fn delete_with_shell_for_idx(&mut self, idx: Option<usize>) { ... }
    pub fn show_properties_for_idx(&mut self, idx: Option<usize>) { ... }
    pub fn create_new_folder(&mut self) { ... }
    pub fn rename_with_shell(&mut self, idx: usize) { ... }
    pub fn create_shell_shortcut(&self, target: &Path) -> Result<PathBuf, String> { ... }
}
```

#### 3.2 `recycle_bin_ops.rs` (Linhas 66-157)

```rust
// src/app/operations/recycle_bin_ops.rs
//! Recycle bin operations: restore, delete permanently, empty

use std::path::Path;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn restore_from_recycle_bin(&mut self, physical_path: &Path) { ... }
    pub fn delete_permanently(&mut self, physical_path: &Path) { ... }
    pub fn empty_recycle_bin(&mut self) { ... }
}
```

#### 3.3 `clipboard_ops.rs` (Linhas 211-266, 2205-2211)

```rust
// src/app/operations/clipboard_ops.rs
//! Clipboard operations: copy, cut, paste, copy path

use std::path::Path;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn command_copy(&mut self, idx: Option<usize>) { ... }
    pub fn command_cut(&mut self, idx: Option<usize>) { ... }
    pub fn command_paste(&mut self, idx: Option<usize>) { ... }
    pub fn copy_path_to_clipboard(&self, path: &Path) { ... }
}
```

#### 3.4 `navigation.rs` (Linhas 560-701, 922-938, 1017-1025)

```rust
// src/app/operations/navigation.rs
//! Navigation: navigate_to, go_back, go_forward, go_up

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn navigate_to(&mut self, path: &str) { ... }
    pub fn go_back(&mut self) { ... }
    pub fn go_forward(&mut self) { ... }
    pub fn navigate_to_computer(&mut self) { ... }
    pub fn navigate_to_recycle_bin(&mut self) { ... }
    pub fn go_up_one_level(&mut self) { ... }
    pub fn can_go_back(&self) -> bool { ... }
    pub fn can_go_forward(&self) -> bool { ... }
}
```

#### 3.5 `folder_loading.rs` (Linhas 267-355, 361-559, 868-878)

```rust
// src/app/operations/folder_loading.rs
//! Folder loading: load_folder, filter_items, sort_items, refresh

use std::path::PathBuf;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn filter_items(&mut self) { ... }
    pub fn sort_items(&mut self) { ... }
    pub fn request_folder_scan(&self, folder_path: PathBuf) { ... }
    pub fn load_folder(&mut self, force_refresh: bool) { ... }
    pub fn trigger_manual_refresh(&mut self) { ... }
}
```

#### 3.6 `view_setup.rs` (Linhas 718-867)

```rust
// src/app/operations/view_setup.rs
//! View setup: computer view, recycle bin view, drive list

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn setup_recycle_bin_view(&mut self) { ... }
    pub fn setup_computer_view(&mut self) { ... }
    pub fn reload_drive_list(&mut self) -> bool { ... }
    pub fn refresh_drives_if_needed(&mut self) { ... }
}
```

#### 3.7 `tabs.rs` (Linhas 879-921)

```rust
// src/app/operations/tabs.rs
//! Tab synchronization

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn sync_to_tab(&mut self) { ... }
    pub fn sync_from_tab(&mut self) { ... }
}
```

#### 3.8 `watcher.rs` (Linhas 939-975)

```rust
// src/app/operations/watcher.rs
//! File system watcher management

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn watch_current_folder(&mut self) { ... }
}
```

#### 3.9 `preferences.rs` (Linhas 286-355)

```rust
// src/app/operations/preferences.rs
//! User preferences save/load

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn save_preferences(&self) { ... }
}
```

#### 3.10 `thumbnails.rs` (Linhas 1026-1040)

```rust
// src/app/operations/thumbnails.rs
//! Thumbnail loading requests

use std::path::PathBuf;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn request_thumbnail_load(&self, path: PathBuf) { ... }
    pub fn request_folder_preview_load(&mut self, path: PathBuf) { ... }
}
```

#### 3.11 `icons.rs` (Linhas 1140-1159)

```rust
// src/app/operations/icons.rs
//! Icon loading and caching

use eframe::egui;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn ensure_folder_icon(&mut self, ctx: &egui::Context) { ... }
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) { ... }
}
```

#### 3.12 `metadata.rs` (Linhas 1160-1235)

```rust
// src/app/operations/metadata.rs
//! Media metadata handling

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn refresh_selected_metadata(&mut self) { ... }
}

// Helper functions (module-private)
fn format_media_duration(ticks_100ns: u64) -> String { ... }
fn format_bitrate(bps: u32) -> String { ... }
fn approximate_bitrate(size_bytes: u64, duration_100ns: u64) -> Option<u32> { ... }
```

#### 3.13 `selection.rs` (Linhas 2154-2187)

```rust
// src/app/operations/selection.rs
//! Selection state management

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn update_selected_thumbnail(&mut self) { ... }
    pub fn reset_selection_and_search(&mut self) { ... }
}
```

#### 3.14 `context_menu.rs` (Linhas 2188-2461)

```rust
// src/app/operations/context_menu.rs
//! Context menu population

use std::path::PathBuf;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn context_target_path(&self, item_idx: Option<usize>) -> Option<PathBuf> { ... }
    pub fn populate_context_menu(...) { ... }
}
```

#### 3.15 `window.rs` (Linhas 1041-1079)

```rust
// src/app/operations/window.rs
//! Window handle management

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn ensure_window_handle(&mut self, _frame: &eframe::Frame) { ... }
}
```

#### 3.16 `message_handler.rs` (Linhas 1236-1542)

```rust
// src/app/operations/message_handler.rs
//! Async message processing from workers

use eframe::egui;
use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    pub fn process_incoming_messages(&mut self, ctx: &egui::Context) { ... }
}
```

### Passo 4: Mover render functions para `ui/`

As funções `render_list_view`, `render_grid_view`, `render_item_slot` (linhas 1543-2102) **NÃO** pertencem a `app/`. Devem ser movidas para:

- `src/ui/views/list_view.rs` - já existe, integrar
- `src/ui/views/grid_view.rs` - já existe, integrar
- `src/ui/components/item_slot.rs` - já existe, integrar

**OU** criar wrappers que delegam para os módulos existentes.

### Passo 5: Remover código duplicado

Linhas 2103-2153 contêm closures duplicadas que já existem como métodos. **DELETAR**.

### Passo 6: Atualizar `src/app/mod.rs`

```rust
//! Main Application Module

pub mod init;
pub mod operations;  // Agora é um diretório!
pub mod state;

pub use state::ImageViewerApp;
```

### Passo 7: Deletar `src/app/message_handler.rs` antigo

O arquivo `src/app/message_handler.rs` atual está vazio e será substituído pelo novo em `operations/`.

### Passo 8: Atualizar imports onde necessário

Verificar se algum módulo externo importa de `app::operations` diretamente.

---

## ✅ Checklist de Verificação

### Durante Execução
- [ ] Criar `src/app/operations/` diretório
- [ ] Criar `src/app/operations/mod.rs`
- [ ] Extrair `file_ops.rs`
- [ ] Extrair `recycle_bin_ops.rs`
- [ ] Extrair `clipboard_ops.rs`
- [ ] Extrair `navigation.rs`
- [ ] Extrair `folder_loading.rs`
- [ ] Extrair `view_setup.rs`
- [ ] Extrair `tabs.rs`
- [ ] Extrair `watcher.rs`
- [ ] Extrair `preferences.rs`
- [ ] Extrair `thumbnails.rs`
- [ ] Extrair `icons.rs`
- [ ] Extrair `metadata.rs`
- [ ] Extrair `selection.rs`
- [ ] Extrair `context_menu.rs`
- [ ] Extrair `window.rs`
- [ ] Extrair `message_handler.rs`
- [ ] Mover/integrar render functions
- [ ] Remover código duplicado
- [ ] Atualizar `app/mod.rs`
- [ ] Deletar `app/message_handler.rs` antigo
- [ ] Deletar `app/operations.rs` original

### Após Cada Módulo
- [ ] `cargo check` passa
- [ ] Sem erros de compilação

### Validação Final
- [ ] `cargo build --release` sucesso
- [ ] `cargo test` passa (se houver testes)
- [ ] Executar app - todas features funcionam
- [ ] Nenhum arquivo > 400 linhas em `app/operations/`

---

## ⚠️ Riscos e Mitigações

| Risco | Probabilidade | Impacto | Mitigação |
|-------|---------------|---------|-----------|
| Imports circulares | Baixa | Alto | Todos os módulos importam apenas de `app::state` |
| Código duplicado não removido | Média | Baixo | Verificar com grep antes de finalizar |
| Render functions mal integradas | Média | Médio | Testar visualmente após mover |
| Esquecimento de algum método | Baixa | Baixo | Comparar contagem de funções antes/depois |

---

## 📊 Métricas de Sucesso

| Métrica | Antes | Depois |
|---------|-------|--------|
| `app/operations.rs` | 2461 linhas | **0** (deletado) |
| Maior arquivo em `app/operations/` | N/A | < 400 linhas |
| Número de módulos | 1 | 16 |
| Média linhas/módulo | 2461 | ~120 |

---

## 🔄 Ordem de Execução Recomendada

1. **Módulos independentes primeiro** (sem dependências internas):
   - `preferences.rs`
   - `thumbnails.rs`
   - `watcher.rs`
   - `tabs.rs`
   - `window.rs`
   - `icons.rs`

2. **Módulos com lógica simples**:
   - `selection.rs`
   - `clipboard_ops.rs`
   - `file_ops.rs`
   - `recycle_bin_ops.rs`

3. **Módulos com lógica complexa**:
   - `navigation.rs`
   - `folder_loading.rs`
   - `view_setup.rs`
   - `metadata.rs`

4. **Módulos grandes**:
   - `message_handler.rs`
   - `context_menu.rs`

5. **Cleanup final**:
   - Render functions
   - Código duplicado
   - Arquivo original

---

## 📁 Template de Arquivo

Cada arquivo novo deve seguir este padrão:

```rust
//! [Descrição breve do módulo]
//!
//! This module handles [responsabilidade específica].

use std::path::{Path, PathBuf};
// ... outros imports necessários

use crate::app::state::ImageViewerApp;

impl ImageViewerApp {
    /// [Documentação da função]
    pub fn nome_da_funcao(&mut self, ...) {
        // Implementação copiada de operations.rs
    }
}
```

---

**Documento preparado para handoff ao agente de implementação.**

**Nota**: Este plano pode ser executado em partes. Cada módulo extraído é um commit válido.
