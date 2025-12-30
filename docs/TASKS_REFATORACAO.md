# Refatoração MTT File Manager - Task List

## Fase 1: Integrar Tipos Existentes
- [x] Analisar quais tipos estão duplicados em `main.rs`
- [x] Remover `SortMode`, `ViewMode`, `IconSize` duplicados de `main.rs` (já feito anteriormente)
- [x] Adicionar imports de `crate::domain` (já existente nas linhas 29-31)
- [x] Verificar compilação (tipos já integrados)

## Fase 2: Extrair Windows APIs ✅
- [x] Criar estrutura base de `windows_api.rs` com todas as funções (686 linhas)
- [x] Atualizar imports em main.rs (`use ... as win_api`)
- [x] Substituir todas as chamadas de funções locais por `win_api::`
- [x] Verificar compilação (✓ `cargo build --release` passou)
- [ ] ~~Remover funções locais duplicadas~~ (deixado para limpeza posterior)

> **Status**: Build passou com 12 warnings de dead code (funções locais não utilizadas).
> Funções migradas e funcionando via `win_api::` module.

## Fase 3: Extrair Workers
- [ ] Implementar `thumbnail_loader.rs`
- [ ] Implementar `folder_scanner.rs`
- [ ] Criar `message_handler.rs`
- [ ] Verificar compilação e funcionamento

## Fase 4: Extrair UI Components
- [ ] Implementar `ui/grid.rs`
- [ ] Criar `ui/list.rs`
- [ ] Criar `ui/item_slot.rs`
- [ ] Implementar `ui/sidebar.rs`
- [ ] Implementar `ui/header.rs`
- [ ] Criar `ui/context_menu.rs`
- [ ] Criar `ui/computer_view.rs`
- [ ] Verificar compilação e funcionamento

## Fase 5: Refatorar App Core
- [ ] Criar `ui/app.rs` com FileManagerApp
- [ ] Mover struct e impls
- [ ] Reduzir `main.rs` para ~50-100 linhas
- [ ] Verificar compilação e funcionamento

## Fase 6: Documentação & Regras
- [ ] Criar `docs/CODING_STANDARDS.md`
- [ ] Atualizar `docs/ARQUITETURA.md`
- [ ] Atualizar `docs/ROADMAP_TECNICO.md`
- [ ] Revisar `.cursorrules`
