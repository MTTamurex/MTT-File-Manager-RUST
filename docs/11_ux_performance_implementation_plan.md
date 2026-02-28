# 11. Plano de Implementação — UX e Performance (egui)

## Objetivo Principal
Prioridade máxima na experiência do usuário:
- UI não pode travar/congelar.
- Navegação deve ser fluida e suave.
- Aparência final dos itens deve aparecer o mais rápido possível.

## Regras Rígidas
- Carregamento de thumbnails, previews, ícones de arquivo e ícones de pasta deve ser assíncrono (background).
- Proibido uso de placeholders visuais (sem imagens temporárias genéricas).
- A thread de UI não deve executar I/O bloqueante, nem locks longos, nem processamento pesado sem orçamento por frame.

## Estado Atual (Hotspots)
- Loop principal de frame e pipeline de mensagens/eventos:
  - `src/ui/app_impl.rs`
  - `src/app/operations/message_handler/mod.rs`
  - `src/app/operations/message_handler/thumbnail_events.rs`
- Escrita de preferência no fluxo de frame:
  - `src/app/operations/message_handler/thumbnail_uploads.rs`
  - `src/infrastructure/disk_cache/preferences.rs`
- Drenagem agressiva de filas/eventos sem limite estrito em alguns caminhos:
  - `src/app/operations/message_handler/file_op_events.rs`
  - `src/app/operations/message_handler/watcher_legacy.rs`

## Itens Herdados da Auditoria (Válidos e Não Conflitantes)
Os itens abaixo foram identificados anteriormente e continuam válidos com o novo critério de sucesso (UX máxima):
- Correções de estabilidade/recursos nativos (sem impacto negativo de UX):
   - Corrigir caminho de erro com potencial vazamento de recursos Win32/COM em menu shell nativo.
   - Balancear corretamente inicialização/finalização COM em rotinas de warmup.
- Robustez de pipeline assíncrono:
   - Evitar flags presas em cenários de falha/desconexão de worker (ex.: scans pendentes).
   - Garantir fallback/retry previsível quando canal desconecta.
- Performance estrutural que ajuda UX:
   - Reduzir clones/alocações no hot path de render/sincronização de abas.
   - Reduzir varreduras O(n) quando updates são localizados por path.
- Hardening progressivo:
   - Remover `unwrap/expect` de runtime em caminhos críticos de produção.
   - Manter `unsafe` encapsulado com invariantes explícitas e liberação por RAII.

### Itens Herdados Fora de Escopo Imediato (mas recomendados)
- Ajustes de baixo impacto em UX que não atacam jank diretamente podem ser executados após P1.
- Refatorações amplas sem ganho mensurável de fluidez devem ficar para fase posterior.

---

## Roadmap Prioritário

### P0 — Eliminar travamentos da UI (Maior impacto / Menor esforço)
1. Remover I/O bloqueante da thread de UI
   - Substituir gravações diretas por batch assíncrono (`try_set_preferences_batch`) com flush periódico.
2. Aplicar orçamento estrito por frame para todos os drains de receiver
   - Limitar por tempo + quantidade em `file_op_events` e `watcher_legacy` (mesmo padrão já usado em `thumbnail_events`).
3. Corrigir inconsistências de throughput no pipeline de ícones
   - Ajustar contadores de processamento por frame para não reduzir uploads indevidamente.
4. Robustez de estados pendentes
   - Garantir reset de flags de worker em casos de falha/desconexão para evitar estados presos.
5. Correções críticas de estabilidade nativa (sem regressão de UX)
   - Corrigir possíveis leaks de recursos (Win32/COM) e garantir cleanup por RAII em caminhos de erro.
   - Balancear COM init/uninit em rotinas de inicialização auxiliar.

### P1 — Fluidez e “final appearance first”
1. Priorização absoluta de visíveis/selecionado
   - Itens visíveis e selecionado sempre têm prioridade de upload de textura e processamento.
2. Fairness entre filas de mídia
   - Evitar starvation entre thumbs, ícones, metadata e folder preview com scheduler por budget.
3. Política “no placeholders” em toda UI principal
   - Grid/list/preview exibem somente dados finais disponíveis (sem assets temporários genéricos).

### P2 — Redução de custo no hot path
1. Reduzir clones/alocações em render e troca de abas
   - Revisar `tabs`, `list_bridge`, `grid_bridge` para reduzir cópias grandes no caminho crítico.
2. Atualizações direcionadas por índice/lookup
   - Evitar varreduras O(n) completas quando atualização for localizada por path.
3. Menos churn de strings/chaves de textura
   - Reuso de chaves e buffers para reduzir pressão de alocador.
4. Hardening de runtime em caminhos críticos
   - Substituir `unwrap/expect` de produção por tratamento de erro controlado e fallback seguro.

### P4 — Estabilidade Estrutural (sem conflitar com UX)
1. Blindagem de FFI/`unsafe`
   - Revisar invariantes, ownership e cleanup em integrações nativas; padronizar guardas RAII.
2. Confiabilidade de workers
   - Política única para reconexão/retentativa, timeout e recuperação em falhas de canal.
3. Robustez orientada a produção
   - Logs acionáveis para falhas de infraestrutura sem degradar o frame loop.

### P3 — Observabilidade e validação contínua
1. Telemetria de UX
   - `frame time p50/p95/p99`, backlog por fila, dropped events, tempo para aparência final por item.
2. Alertas operacionais
   - Thresholds para regressões de fluidez e crescimento de backlog.
3. Regressão automatizada (stress)
   - Cenários com flood de FS events, navegação rápida e carga de mídia pesada.

---

## Critérios de Aceite
- Sem congelamento perceptível durante navegação e operações de arquivo.
- Scroll e navegação fluidos sob carga.
- Carregamento de ícones/thumbnails/previews em background sem bloquear a UI.
- Sem placeholders visuais no fluxo principal.
- Tempo para “aparência final” reduzido e estável em cenários reais.

## Ordem Recomendada de Execução
1. P0 completo.
2. P1 completo.
3. P2 incremental por módulo.
4. P3 para sustentação e prevenção de regressão.
5. P4 para robustez de longo prazo.
