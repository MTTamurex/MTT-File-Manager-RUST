# Plano de Refatoração de Módulos Monolíticos
Data: 2026-02-08  
Projeto: MTT File Manager (Rust)

## Objetivo
Reduzir acoplamento e custo de manutenção, quebrando arquivos com múltiplas responsabilidades em módulos menores, coesos e testáveis, sem alterar comportamento funcional.

## Critérios usados na auditoria
1. Tamanho do arquivo (LOC alto).  
2. Quantidade de responsabilidades no mesmo módulo.  
3. Presença de função central muito longa (ex.: um `pub fn` com centenas de linhas).  
4. Mistura de camadas (UI + regras de domínio + infra no mesmo fluxo).  
5. Dificuldade de teste isolado.

## Diagnóstico (monólitos identificados)
### Prioridade Alta
1. `src/app/operations/message_handler.rs` (1268 linhas, núcleo de eventos assíncronos em um único fluxo).
2. `src/app/operations/folder_loading.rs` (961 linhas, mistura load/filter/sort/refresh/cover scan).
3. `src/ui/views/grid_view.rs` (924 linhas, render + virtualização + scroll + prefetch + interação).
4. `src/app/init.rs` (767 linhas, bootstrap completo em `new()` com alto acoplamento).

### Prioridade Média
1. `src/ui/app_impl.rs` (797 linhas, loop principal + camadas + orchestration densa).
2. `src/ui/tab_bar.rs` (540 linhas, renderização e lógica de interação concentradas).
3. `src/infrastructure/onedrive.rs` (598 linhas, utilitários + timeout infra + enumeração + status).
4. `src/ui/components/mpv_preview.rs` (769 linhas, estado + ciclo de vida + bridge com mpv).
5. `src/ui/components/item_slot.rs` (650 linhas, múltiplos tipos de slot e regras visuais).

### Observação
`src/infrastructure/windows/codec_registry.rs` é grande (833 linhas), mas é majoritariamente mapeamento/lookup de codec na mesma responsabilidade; não entra como prioridade de quebra estrutural agora.

---

## Plano de Refatoração (fases)
## Fase 0 - Baseline e Segurança
1. Congelar baseline de comportamento (build release + smoke tests manuais do fluxo principal).  
2. Garantir checklist de regressão para cada PR (navegação, seleção, preview, operações de arquivo, watchers).  
3. Refatorar em passos pequenos (sem big-bang).

## Fase 1 - Extrair monólitos críticos de operação
### 1. `message_handler.rs`
Separar por tipo de evento:
1. `src/app/operations/message_handler/core.rs` (orquestrador curto).
2. `src/app/operations/message_handler/file_ops.rs`.
3. `src/app/operations/message_handler/watcher_events.rs`.
4. `src/app/operations/message_handler/thumbnail_events.rs`.
5. `src/app/operations/message_handler/rebuild_events.rs`.
6. `src/app/operations/message_handler/helpers.rs` (normalização/comparação path).

Meta: `process_incoming_messages()` virar pipeline curto de dispatch.

### 2. `folder_loading.rs`
Separar responsabilidade de carregamento:
1. `src/app/operations/folder_loading/load_pipeline.rs`.
2. `src/app/operations/folder_loading/folder_scan.rs`.
3. `src/app/operations/folder_loading/refresh.rs`.
4. `src/app/operations/folder_loading/guards.rs`.
5. `src/app/operations/folder_loading/view_updates.rs`.

Meta: reduzir função `load_folder()` para coordenação, removendo detalhes internos.

## Fase 2 - Refatoração de UI pesada
### 3. `grid_view.rs`
Adotar estrutura semelhante à list view modular:
1. `src/ui/views/grid_view/mod.rs`.
2. `src/ui/views/grid_view/virtualization.rs`.
3. `src/ui/views/grid_view/item_renderer.rs`.
4. `src/ui/views/grid_view/scroll.rs`.
5. `src/ui/views/grid_view/prefetch.rs`.
6. `src/ui/views/grid_view/interactions.rs`.

Meta: reduzir arquivo principal para composição e tipos públicos.

### 4. `app_impl.rs`
Extrair etapas do update loop:
1. `src/ui/app/update_loop.rs`.
2. `src/ui/app/layers/status_bar_layer.rs`.
3. `src/ui/app/layers/tab_bar_layer.rs`.
4. `src/ui/app/layers/toolbar_layer.rs`.
5. `src/ui/app/layers/secondary_toolbar_layer.rs`.

Meta: `eframe::App::update()` com fluxo legível e sem blocos extensos.

### 5. `tab_bar.rs`
Quebrar em:
1. `src/ui/tab_bar/mod.rs`.
2. `src/ui/tab_bar/tabs_renderer.rs`.
3. `src/ui/tab_bar/window_controls.rs`.
4. `src/ui/tab_bar/drag_dwell.rs`.

Meta: separar desenho de aba, ações e controles de janela.

## Fase 3 - Infra/Componentes de suporte
### 6. `onedrive.rs`
Separar utilitários:
1. `src/infrastructure/onedrive/path_detection.rs`.
2. `src/infrastructure/onedrive/attributes.rs`.
3. `src/infrastructure/onedrive/timeout_ops.rs`.
4. `src/infrastructure/onedrive/directory_enum.rs`.
5. `src/infrastructure/onedrive/mod.rs`.

### 7. `mpv_preview.rs`
Separar bridge do player:
1. `src/ui/components/mpv_preview/mod.rs`.
2. `src/ui/components/mpv_preview/lifecycle.rs`.
3. `src/ui/components/mpv_preview/window_embed.rs`.
4. `src/ui/components/mpv_preview/playback_state.rs`.

### 8. `item_slot.rs`
Separar por tipo de item:
1. `src/ui/components/item_slot/mod.rs`.
2. `src/ui/components/item_slot/drive_slot.rs`.
3. `src/ui/components/item_slot/folder_slot.rs`.
4. `src/ui/components/item_slot/file_slot.rs`.
5. `src/ui/components/item_slot/badges.rs`.

---

## Ordem recomendada de execução
1. `message_handler.rs`  
2. `folder_loading.rs`  
3. `grid_view.rs`  
4. `app_impl.rs`  
5. `tab_bar.rs`  
6. `onedrive.rs`  
7. `mpv_preview.rs`  
8. `item_slot.rs`

Racional: começa nos pontos com maior risco de regressão silenciosa e maior ganho em legibilidade/testabilidade.

## Estratégia de execução por módulo
1. Criar submódulos e mover código sem alterar assinatura pública.  
2. Compilar (`cargo check`) após cada extração parcial.  
3. Validar fluxo funcional mínimo daquele domínio.  
4. Somente depois limpar imports/helpers redundantes.  
5. Evitar refatorar múltiplos domínios no mesmo commit.

## Critérios de pronto (Definition of Done)
1. Comportamento preservado no fluxo funcional do módulo.  
2. `cargo check` verde após cada etapa.  
3. Redução de LOC no arquivo original (arquivo coordenador menor).  
4. Sem aumento de acoplamento entre camadas.  
5. Logs e tratamento de erro preservados.

## Riscos e mitigação
1. Regressão em fluxo assíncrono: manter extrações pequenas e validar canal por canal.  
2. Regressão de UI: validar interação (click/double/right/drag) após cada fase.  
3. Regressão de performance: não alterar algoritmos na mesma etapa da extração estrutural.

---

## Status
Plano criado. Nenhuma refatoração iniciada ainda.  
Aguardando sua ordem para começar pela Fase 1.
