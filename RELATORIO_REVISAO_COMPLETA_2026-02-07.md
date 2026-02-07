# Relatorio Tecnico de Revisao Completa - MTT File Manager (Rust)

Data: 2026-02-07  
Projeto: `MTT-File-Manager-RUST`

## 1. Escopo e metodologia

Esta revisao foi feita com foco em:
- Estabilidade geral da aplicacao
- Responsividade da UI (loop principal, processamento de eventos, frame time)
- Experiencia do usuario (tempo para thumbnails visiveis e comportamento do preview)
- Otimizacao de I/O para HDD mecanico e cenarios OneDrive

### Comandos executados
- `cargo check -q`: OK
- `cargo test -q`: OK (`68 passed; 0 failed`)
- `cargo clippy -q`: OK com warnings (`167` warnings)

### Cobertura tecnica analisada
- Pipeline de thumbnails (`queue`, `worker`, `upload GPU`, caches RAM/SQLite)
- Watchers de filesystem (DriveWatcher + fallback notify)
- Carregamento de pasta, cache de diretorio e index
- Integracao OneDrive com timeouts
- Loop principal da UI e processamento de mensagens

## 2. Resumo executivo

O projeto tem base tecnica forte e varias decisoes corretas para performance (cache-first, batching, invalidacao por watcher, throttling de upload GPU, prioridade de I/O). Entretanto, encontrei alguns pontos de risco alto/critico que afetam diretamente os objetivos que voce destacou (estabilidade, responsividade e menor espera para thumbnails):

1. **Filtro por pasta no DriveWatcher nao esta efetivamente aplicado**, gerando processamento global do drive.  
2. **Wrappers de timeout do OneDrive criam threads destacadas por timeout**, com contagem de concorrencia inconsistente.  
3. **Fila de thumbnails deduplica por path sem promover prioridade/tamanho**, podendo atrasar thumbnail visivel e preview em alta resolucao.  
4. **Deteccao SSD/HDD da fila e global e definida so na primeira requisicao**, comprometendo otimizações em ambiente multi-drive.  
5. **Transicao de prioridade de thread (background -> interativo) esta incompleta**, reduzindo efeito das prioridades interativas.

Esses 5 itens sao os mais importantes para atacar primeiro.

## 3. Pontos fortes atuais

- Boa separacao de camadas e modulos (app/application/domain/infrastructure/ui/workers).
- `generation`/cancelamento para evitar resultados stale entre navegacoes.
- Cache-first no worker de thumbnail antes de tocar origem (`src/workers/thumbnail/worker.rs:181`).
- Pipeline de upload GPU com limite por frame e budget adaptativo (`src/app/operations/message_handler.rs:1063`).
- Estrategias especificas para HDD no carregamento de pasta (NTFS nativo, fallback otimizado) (`src/app/operations/folder_loading.rs:497`).
- Timeouts explicitos para OneDrive e diversas protecoes anti-bloqueio.
- Suite de testes executou com sucesso.

## 4. Achados priorizados

## [C1] Critico - DriveWatcher sem filtro efetivo por prefixo atual

### Evidencia
- O modulo documenta filtro por prefixo atual (`src/infrastructure/drive_watcher.rs:70`).
- `UpdatePrefix` e recebido mas ignorado no thread (`src/infrastructure/drive_watcher.rs:282`).
- Nao ha filtro de eventos por prefixo no loop principal do watcher.
- `event_matches_prefix` existe apenas em `#[cfg(test)]` (`src/infrastructure/drive_watcher.rs:453`).

### Impacto
- Eventos de todo o drive entram no pipeline da UI.
- Mais invalidacoes e mais trabalho no `process_incoming_messages`.
- Maior chance de stutter em maquinas com atividade intensa fora da pasta atual.

### Recomendacao
- Aplicar filtro de prefixo dentro do thread do watcher antes de coalescer/enviar lote.
- Manter prefixo normalizado compartilhado entre `update_prefix` e thread.
- Enviar para UI somente eventos relevantes para a view ativa.

## [C2] Critico - Timeout OneDrive com thread destacada e contagem inconsistente

### Evidencia
- `metadata_with_timeout`: spawn thread (`src/infrastructure/onedrive.rs:347`) e, em timeout, sai sem `join` (`src/infrastructure/onedrive.rs:357`), mas decrementa contador imediatamente (`src/infrastructure/onedrive.rs:381`).
- Mesmo padrao em `exists_with_timeout` (`src/infrastructure/onedrive.rs:434`, `src/infrastructure/onedrive.rs:444`, `src/infrastructure/onedrive.rs:464`).
- Mesmo padrao em `read_directory_with_timeout` (`src/infrastructure/onedrive.rs:551`, `src/infrastructure/onedrive.rs:561`, `src/infrastructure/onedrive.rs:582`).

### Impacto
- Em rajadas de timeout, pode acumular threads bloqueadas no SO.
- Contador de concorrencia (`ACTIVE_TIMEOUT_THREADS`) deixa de representar threads reais em execucao.
- Risco de degradacao progressiva e instabilidade sob cenarios ruins de OneDrive.

### Recomendacao
- Trocar spawn por chamada para pool fixa (bounded) de workers de I/O OneDrive.
- Usar fila com limite e descarte controlado para evitar explosao de threads.
- So decrementar contagem quando operacao realmente finalizar.

## [H1] Alto - Deduplicacao da fila ignora upgrade de prioridade/tamanho

### Evidencia
- Se o path ja esta pendente, a fila retorna sem atualizar request (`src/workers/thumbnail/queue.rs:76`).
- O app pede thumbnail 512 para item selecionado (`src/app/operations/selection.rs:53`).
- Prefetch em list view usa tamanho menor (`src/ui/views/list_view/virtualization.rs:429`).

### Impacto
- Requisicao interativa pode ficar "atras" de prefetch antigo do mesmo arquivo.
- Preview pode demorar para subir de qualidade quando o usuario seleciona rapidamente.
- UX de "thumbnail borrado por tempo maior que necessario".

### Recomendacao
- Ao receber path duplicado, fazer merge da request:
  - `priority = min(priority_atual, nova_priority)`
  - `size = max(size_atual, novo_size)`
  - atualizar `directory_index` quando aplicavel

## [H2] Alto - Modo SSD/HDD da fila e global e fixado pela primeira request

### Evidencia
- Estado unico `is_ssd: Option<bool>` (`src/workers/thumbnail/queue.rs:20`).
- Detecta apenas na primeira insercao (`src/workers/thumbnail/queue.rs:81`).

### Impacto
- Em sessao multi-drive (ex.: C: SSD, D: HDD), a estrategia pode ficar errada para parte das requests.
- Piora localidade em HDD ou adiciona ordenacao desnecessaria em SSD.

### Recomendacao
- Classificar por drive e nao por fila global.
- Opcoes:
  - subfilas por drive
  - scheduler central com politica por request

## [H3] Alto - Prioridade de thread pode permanecer em background

### Evidencia
- `THREAD_MODE_BACKGROUND_BEGIN` e aplicado em background (`src/infrastructure/io_priority.rs:287`).
- Para `Interactive`/`Prefetch`, nao ha `THREAD_MODE_BACKGROUND_END` antes de subir prioridade (`src/infrastructure/io_priority.rs:275`).
- Worker seta background na inicializacao (`src/workers/thumbnail/worker.rs:120`) e depois muda por request (`src/workers/thumbnail/queue.rs:148`).

### Impacto
- Requests interativas podem continuar com tratamento de I/O de background.
- Tempo de resposta pior para thumbnails visiveis no viewport.

### Recomendacao
- Ao trocar de `Background` para `Prefetch/Interactive`, chamar `THREAD_MODE_BACKGROUND_END`.
- Guardar estado atual de prioridade por thread para transicoes consistentes.

## [H4] Alto - Invalidacao de cache em SQL no thread da UI durante eventos

### Evidencia
- Em DELETE do watcher, chama `remove_cache_for_path` no fluxo de mensagens da UI (`src/app/operations/message_handler.rs:508`).
- `remove_cache_for_path` executa multiplas queries SQL por evento (`src/infrastructure/disk_cache.rs:415`).

### Impacto
- Pode causar travamento perceptivel mesmo abaixo do limiar de flood.
- Afeta frame-time em diretorios com mudancas frequentes.

### Recomendacao
- Migrar invalidacao SQL para worker dedicado com batch e transacao.
- Deixar na UI apenas invalidações leves em memoria e sinalizacao de reload.

## [M1] Medio-Alto - Log flood no startup enquanto watcher esta em delay

### Evidencia
- Delay de 5s para ativacao inicial (`src/infrastructure/drive_watcher_integration.rs:45`).
- `poll_events` loga toda chamada sem drive ativo (`src/infrastructure/drive_watcher_integration.rs:125`).

### Impacto
- Escreve log excessivo no inicio (ate dezenas por segundo), gerando I/O inutil.

### Recomendacao
- Remover log por-frame ou aplicar rate-limit (ex.: 1 log a cada 5s no maximo).

## [M2] Medio - GC de cache agressivo cedo e com `Path::exists`

### Evidencia
- GC inicia apos 3s do startup (`src/app/init.rs:764`).
- GC verifica existencia com `Path::exists()` para todos registros (`src/infrastructure/disk_cache.rs:498`).
- Executa `VACUUM` quando remove entradas (`src/infrastructure/disk_cache.rs:559`).

### Impacto
- Contencao de I/O no inicio, inclusive em HDD.
- Em caminhos OneDrive, risco de latencia maior.

### Recomendacao
- Executar GC em janela de idle real.
- Fazer em lotes pequenos incrementais.
- Adiar `VACUUM` para manutencao periodica (ou threshold maior).

## [M3] Medio - Status OneDrive "LocallyAvailable" presumido para cache

### Evidencia
- Entradas em cache sao forçadas para `LocallyAvailable` em alguns fluxos (`src/app/operations/folder_loading.rs:277`, `src/app/operations/folder_loading.rs:457`).

### Impacto
- Usuario pode ver status incorreto ate o scan fresco concluir.

### Recomendacao
- Usar estado neutro (ex.: desconhecido) para dados vindos apenas de cache.
- Atualizar status quando chegar leitura fresca.

## [L1] Baixo - Divida tecnica de manutencao (Clippy)

### Evidencia
- `cargo clippy -q` retornou `167` warnings.
- Predominam: funcoes com muitos argumentos, `&PathBuf` em vez de `&Path`, `new_without_default`, simplificacoes de fluxo.

### Impacto
- Nao quebra runtime no curto prazo, mas reduz velocidade de evolucao e aumenta risco de regressao.

### Recomendacao
- Criar baseline de warnings e reduzir gradualmente por modulo critico (thumb/watcher/folder loading/UI loop).

## 5. Otimizacoes recomendadas (foco HDD + UX)

1. Corrigir dedup da fila com merge de prioridade/tamanho (ganho direto na UX de thumbnails visiveis).  
2. Aplicar filtro de prefixo no DriveWatcher antes da UI (menos invalidação e menos I/O indireto).  
3. Mover invalidacao SQL de eventos para worker batch transacional (menos stutter).  
4. Reescrever timeout OneDrive para pool bounded (estabilidade sob degradacao).  
5. Ajustar transicoes de prioridade de thread (interativo realmente interativo).  
6. Replanejar GC para modo incremental/idle (evitar disputa no startup).

## 6. Plano de execucao sugerido

### Fase 0 (hotfix, 1-2 dias)
- [ ] C1: filtro de prefixo no DriveWatcher
- [ ] H1: merge de request deduplicada na fila
- [ ] H3: conserto de transicao de prioridade background/interativo
- [ ] M1: remover log flood de startup

### Fase 1 (estabilidade forte, 3-5 dias)
- [ ] C2: pool bounded para I/O OneDrive com timeout
- [ ] H2: politica SSD/HDD por drive
- [ ] H4: invalidacao SQL fora da UI

### Fase 2 (otimizacao continua)
- [ ] M2: GC incremental + janela de idle
- [ ] M3: status OneDrive mais correto no cache
- [ ] L1: reduzir warnings Clippy em modulos criticos

## 7. KPIs para validar melhoria

- p95 tempo ate primeira thumbnail visivel na pasta
- p95 tempo de upgrade de thumbnail do item selecionado
- frames > 33ms por minuto (em navegacao e scroll)
- tempo medio de `process_incoming_messages` (`src/app/operations/message_handler.rs`)
- numero maximo de workers OneDrive ativos e timeouts por minuto
- I/O total durante startup (primeiros 10s)

## 8. Conclusao

A arquitetura atual ja contem varias decisoes corretas para alta performance. O maior ganho agora nao esta em adicionar novos mecanismos, e sim em **corrigir alguns pontos de concorrencia/fila/watcher** que estao reduzindo o retorno dessas otimizacoes em cenarios reais (HDD + OneDrive + atividade externa no drive).

Com as correcoes da Fase 0 e Fase 1, a tendencia e melhorar simultaneamente:
- estabilidade (menos risco de degradacao por timeout/thread),
- responsividade (menos trabalho pesado no thread da UI),
- experiencia do usuario (thumbnails prioritarias carregando mais rapido e com menos artefato de qualidade).
