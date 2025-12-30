# Sprint 3 - Continuação da Refatoração

## Status: 🚧 Em Andamento (30/12/2025)

### Progresso Atual:
- ✅ **Fase 1: Windows APIs** - Extraídas e integradas
- ✅ **Fase 2: Views** - Extraídas e integradas (grid_view, list_view, computer_view)
- ⚠️ **Fase 3: Sidebar e Navegação** - Sidebar ainda inline, navigation.rs não integrado
- ❌ **Fase 4: Context Menu** - Ainda no main.rs
- ❌ **Fase 5: CacheManager** - Não integrado
- ❌ **Fase 6: Limpeza** - Arquivos .bak ainda existem

### Métricas Atuais:
- `main.rs`: ~2800 linhas (redução de ~400 linhas com extração das views)
- Views extraídas: 3 módulos funcionais
- Compilação: ✅ Funcionando com warnings

## Objetivo
Continuar a refatoração do `main.rs` (atualmente com ~2736 linhas) para alcançar o limite de 300 linhas, extraindo componentes para módulos dedicados.

---

## Estado Atual (Após Sprint 2)

| Arquivo | Linhas | Status |
|---------|--------|--------|
| `main.rs` | 2736 | ❌ Muito grande |
| `ui/components/item_slot.rs` | 308 | ⚠️ Ligeiramente acima |
| `ui/cache.rs` | 280 | ⚠️ Criado, não integrado |
| `ui/status_bar.rs` | 85 | ✅ Integrado |
| `ui/core.rs` | ~350 | ⚠️ Acima do limite |
| `infrastructure/security.rs` | 262 | ✅ OK |

---

## Tarefas do Sprint 3

### Fase 1: Extrair Windows APIs (~400 linhas)
**Prioridade: Alta** | **Impacto: -400 linhas**

Mover todas as funções de Windows FFI para `infrastructure/windows/`:

- [ ] `extract_windows_thumbnail` → `thumbnail.rs`
- [ ] `hbitmap_to_rgba`, `hicon_to_rgba` → `gdi.rs`
- [ ] `extract_file_icon`, `extract_file_icon_by_path` → `icons.rs`
- [ ] `extract_folder_icon_internal` → `icons.rs`
- [ ] `extract_drive_icon` → `icons.rs`
- [ ] `get_volume_label`, `get_all_drives` → `drives.rs`
- [ ] `open_with_shell` → `shell_operations.rs` (já existe)

**Estrutura alvo:**
```
infrastructure/windows/
├── mod.rs           # Re-exportações
├── gdi.rs           # HBITMAP/HICON → RGBA
├── icons.rs         # Extração de ícones
├── drives.rs        # Enumeração de drives
├── thumbnail.rs     # Extração de thumbnails
└── shell_operations.rs  # Já existe
```

---

### Fase 2: Extrair Views (~600 linhas)
**Prioridade: Alta** | **Impacto: -600 linhas**

- [ ] `render_grid_view` → `ui/views/grid.rs`
- [ ] `render_list_view` → `ui/views/list.rs`
- [ ] Computer View → `ui/views/computer.rs`
- [ ] Preview Panel → `ui/views/preview.rs`

**Estrutura alvo:**
```
ui/views/
├── mod.rs
├── grid.rs          # Grid view (thumbnails)
├── list.rs          # List view (tabela)
├── computer.rs      # "Este Computador"
└── preview.rs       # Painel de preview
```

---

### Fase 3: Extrair Sidebar e Navegação (~300 linhas)
**Prioridade: Média** | **Impacto: -300 linhas**

- [ ] Sidebar com drives → `ui/sidebar.rs` (arquivo existe, está vazio)
- [ ] Navigation bar → `ui/navigation.rs` (arquivo existe, ~110 linhas)
- [ ] Breadcrumb/path bar → integrar em `navigation.rs`

---

### Fase 4: Extrair Context Menu (~150 linhas)
**Prioridade: Média** | **Impacto: -150 linhas**

- [ ] `show_context_menu` → `ui/context_menu.rs`
- [ ] Reativar `ui/context_menu_handling.rs.bak`

---

### Fase 5: Integrar CacheManager
**Prioridade: Baixa** | **Impacto: Qualidade de código**

- [ ] Substituir `texture_cache: LruCache` por `CacheManager`
- [ ] Substituir `icon_cache: LruCache` por `CacheManager`
- [ ] Remover campos redundantes de `ImageViewerApp`
- [ ] Atualizar `ItemSlotContext` para usar `CacheManager`

---

### Fase 6: Limpeza e Arquivos .bak
**Prioridade: Baixa**

- [ ] Avaliar `render_drive_slot.rs.bak` - reimplementar ou deletar
- [ ] Avaliar `texture_cache.rs.bak` - consolidar em `cache.rs`
- [ ] Deletar backups obsoletos (`main.rs.backup`, etc.)
- [ ] Remover warnings (`icon_config` não usado)

---

## Métricas Alvo

| Arquivo | Antes | Depois | Redução |
|---------|-------|--------|---------|
| `main.rs` | 2736 | ~500 | 81% |
| Total novos módulos | - | ~2200 | - |

**Meta final**: `main.rs` com ~500 linhas contendo apenas:
- Struct `ImageViewerApp` 
- `impl eframe::App`
- Função `main()`
- Configuração inicial

---

## Critérios de Sucesso

- [ ] `cargo build --release` sem erros
- [ ] `cargo clippy` sem warnings críticos
- [ ] Todos os arquivos ≤ 300 linhas (exceto views complexas)
- [ ] Aplicação funciona identicamente ao estado anterior
- [ ] Documentação atualizada

---

## Dependências

- Sprint 2 ✅ Concluído (com correções)
- Nenhuma dependência externa nova

---

## Notas Técnicas

### Padrão para Extração de Componentes

1. **Criar módulo** com função standalone
2. **Definir trait** se callback for necessário
3. **Testar compilação** após cada extração
4. **Verificar borrow patterns** antes de integrar

### Exemplo de Padrão Correto
```rust
// Em ui/views/grid.rs
pub fn render_grid_view<O: GridOperations>(
    ui: &mut Ui,
    ctx: &mut GridContext,
    ops: &mut O,
) { ... }

// Trait para callbacks
pub trait GridOperations {
    fn on_item_click(&mut self, idx: usize);
    fn on_item_double_click(&mut self, idx: usize);
}
```

---

## Histórico

| Data | Ação |
|------|------|
| 30/12/2024 | Sprint 3 planejado |
