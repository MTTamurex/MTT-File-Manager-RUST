# Plano: Acesso Rápido — Pastas Fixadas na Sidebar

## Contexto

O usuário quer adicionar a funcionalidade de **Acesso Rápido** à sidebar, semelhante ao Windows Explorer. A sidebar já exibe OneDrive e Lixeira como atalhos fixos na seção "Acesso Rápido". O objetivo é permitir que o usuário fixe/desfixe qualquer pasta e reordene os itens fixados.

**Comportamento esperado:**
- **Fixar**: clique direito na pasta → "Fixar no Acesso Rápido" + arrastar pasta para a seção na sidebar
- **Desafixar**: ícone de pin visível ao hover no item da sidebar — clicar remove
- **Reordenar**: drag-and-drop dentro da seção "Acesso Rápido" na sidebar

---

## Arquitetura

### Novos arquivos (3)

**`src/domain/pinned_folder.rs`**
```rust
#[derive(Debug, Clone)]
pub struct PinnedFolder {
    pub path: String,        // Caminho absoluto
    pub display_name: String, // file_name() do path
    pub position: i64,       // Ordem na lista (0-based)
}
```

**`src/infrastructure/disk_cache/pinned_folders.rs`**
- Segue o padrão de `folder_locks.rs`
- `get_all_pinned_folders(&self) -> Vec<PinnedFolder>` — carrega ordenado por `position`
- `save_pinned_folder(&self, path: &str, name: &str, position: i64)`
- `remove_pinned_folder(&self, path: &str)`
- `update_pinned_positions(&self, ordered_paths: &[String])` — reatribui posições 0..n em batch

**`src/app/operations/pinned_folder_ops.rs`**
- Implementa em `ImageViewerApp`:
  - `pin_folder(&mut self, path: &str)`
  - `unpin_folder(&mut self, path: &str)`
  - `reorder_pinned_folder(&mut self, from: usize, to: usize)`

---

### Arquivos modificados (9)

**1. `src/domain/mod.rs`** — `pub mod pinned_folder;`

**2. `src/infrastructure/disk_cache.rs`**
- `mod pinned_folders;` no topo
- Em `run_migrations()`:
  ```sql
  CREATE TABLE IF NOT EXISTS pinned_folders (
      path TEXT PRIMARY KEY,
      display_name TEXT NOT NULL,
      position INTEGER NOT NULL DEFAULT 0
  )
  ```

**3. `src/app/state.rs`** — campo `pub pinned_folders: Vec<PinnedFolder>`

**4. `src/app/init.rs`** — carregar `disk_cache.get_all_pinned_folders()` na inicialização

**5. `src/app/operations/mod.rs`** — `mod pinned_folder_ops;`

**6. `src/ui/sidebar.rs`**
- `SidebarContext` recebe: `pinned_folders`, `is_folder_dragging`, `dragging_path`
- `SidebarAction` ganha: `PinFolder(String)`, `UnpinFolder(String)`, `ReorderPinnedFolder { from, to }`
- Renderiza itens fixados com ícone nativo, estado selecionado, pin icon ao hover
- Drop zone para fixar via drag da área principal

**7. `src/ui/app/panels.rs`** — passa os novos campos no contexto e trata as novas ações

**8. `src/app/operations/context_menu.rs`** — itens -60 ("Fixar") e -61 ("Remover do Acesso Rápido") para pastas

**9. `src/ui/app/menu_handler.rs`** — handle dos IDs -60 e -61

---

## Verificação

1. Compilar sem erros
2. Clique direito em pasta → "Fixar no Acesso Rápido"
3. Pasta aparece na sidebar; hover mostra pin icon; clicar remove
4. Arrastar pasta para "Acesso Rápido" na sidebar → fixa
5. Reordenar via drag na sidebar
6. Reiniciar → persistência OK
