# Auditoria de Estabilidade, Performance e Segurança Operacional

Data: 2026-04-05
Projeto: MTT File Manager (branch atual: Dark-theme)
Escopo: código-fonte versionado, crates do workspace, scripts e integração Windows. `target/`, outputs gerados e binários em `vendor/` ficaram fora do escopo direto.

## Resumo executivo

O estado atual do projeto é melhor do que o baseline histórico de 2026-04-02 em vários pontos importantes: há mitigação visível em canais bounded do PDF viewer, limite de workers no image viewer, uso de `parking_lot` no search-service, validação do protocolo IPC e backoff melhor no scanner non-USN. Ainda assim, permanecem riscos materialmente relevantes para estabilidade e responsividade no Windows.

Os riscos mais importantes encontrados no HEAD atual são:

1. O encerramento normal da GUI continua usando `TerminateProcess`, o que aborta o cleanup do processo, pode cortar operações de arquivo/cache no meio e invalida praticamente toda a estratégia de shutdown gracioso já implementada.
2. O search-service ainda segura `RwLock` compartilhado durante consultas e checagens de autorização potencialmente lentas, o que degrada atualização incremental, latência de busca e shutdown sob carga real.
3. O painel de preview ainda aceita extração síncrona de ícone na UI thread em misses frios, criando congelamentos visíveis em shell lookups caros.
4. O image viewer continua com rasterização síncrona de SVG sem timeout e com teto alto de 8192 px, o que ainda permite picos grandes de CPU/RAM em arquivos hostis ou simplesmente grandes.

## Bugs confirmados

### 1. Encerramento abrupto do processo no caminho normal de saída

- Severidade: Crítica
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/ui/app/lifecycle.rs` → `handle_exit()`
  - `src/main.rs` → bloco após `eframe::run_native(...)`
  - `src/app/operations/shutdown.rs` → `shutdown_background_workers()` existe, mas não é chamado
- Problema objetivo:
  - O fechamento normal da aplicação chama `TerminateProcess(GetCurrentProcess(), 0)` dentro de `handle_exit()` e, como redundância, há outro caminho de `TerminateProcess` após o retorno de `run_native()`.
  - Isso mata o processo sem esperar término de workers, sem deixar destrutores normais rodarem e sem usar o caminho explícito de shutdown gracioso já escrito em `shutdown_background_workers()`.
- Por que isso afeta estabilidade/performance no Windows:
  - Em Windows, `TerminateProcess` encerra o processo de forma assíncrona e ignora teardown de alto nível. Handles nativos até serão recuperados pelo kernel, mas operações de arquivo, extrações, gravações SQLite, flush de logs, COM/MF teardown, shutdown de MPV e descarte coordenado de filas ficam sujeitos a interrupção abrupta.
  - Isso é especialmente ruim quando o processo ainda possui threads em Shell API, extração de arquivos, escrita de cache, workers de busca ou I/O cancelation.
- Cenário de reprodução:
  - Iniciar cópia/movimentação/exclusão, extração de arquivo, composição de preview ou gravação de preferências e fechar a janela principal.
  - Fechar a janela enquanto há workers ocupados com I/O lento, OneDrive ou shell operations.
- Impacto provável:
  - arquivos parcialmente copiados ou extraídos,
  - preferências e caches não persistidos de forma consistente,
  - comportamento de saída imprevisível,
  - maior chance de “state loss” e pós-condições incorretas.
- Recomendação prática:
  - Trocar o fluxo para shutdown em duas fases:
    1. chamar `shutdown_background_workers()` e sinalizar cancelamento cooperativo,
    2. aguardar um timeout curto e bem definido,
    3. só usar hard-kill como último recurso, e apenas quando o processo realmente estiver travado.
  - Remover o segundo `TerminateProcess` redundante em `main.rs` ou condicioná-lo a um estado explícito de falha de shutdown.

### 2. Consultas do search-service seguram `RwLock` compartilhado durante trabalho lento

- Severidade: Alta
- Confiança: Alta
- Arquivos / funções afetados:
  - `crates/mtt-search-service/src/ipc_server/handler.rs` → `handle_client()`
  - `crates/mtt-search-service/src/ipc_authorization.rs` → `collect_authorized_search_page()` e `collect_authorized_fts_page()`
  - `crates/mtt-search-service/src/file_index.rs` → `search_page()`
- Problema objetivo:
  - `handle_client()` abre `let indices_lock = indices.read();` e mantém esse lock por toda a consulta.
  - Na via linear, `search_page()` pode escanear o índice por até 3 segundos antes de devolver parcial.
  - Na via FTS/autorização, o serviço ainda resolve path e faz `CreateFileW` para checar acesso do cliente enquanto o lock continua vivo.
- Por que isso afeta estabilidade/performance no Windows:
  - Em Windows, `CreateFileW`, path resolution e ACL checks podem ficar lentos por disco frio, AV/minifilter, cloud files, FUSE/WinFsp e árvores grandes.
  - Enquanto o lock de leitura está preso, writers do indexador incremental ficam bloqueados, o índice atrasa e o serviço perde responsividade sob carga.
- Cenário de reprodução:
  - Busca ampla com muitos candidatos, especialmente com AV ativo ou cold cache.
  - Busca concorrente com indexação incremental USN ou persistência periódica.
- Impacto provável:
  - aumento de latência de busca,
  - atraso visível na atualização do índice,
  - maior chance de saturar clientes IPC ativos,
  - shutdown do serviço mais lento.
- Recomendação prática:
  - Tirar a autorização e resolução de caminho de dentro do lock compartilhado.
  - Produzir um snapshot mínimo de candidatos sob lock e soltar o lock antes de `CreateFileW`/autorização.
  - Se necessário, usar snapshots por volume ou estruturas imutáveis compartilhadas por `Arc`.

### 3. Preview panel ainda permite extração síncrona de ícone na UI thread

- Severidade: Alta
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/ui/preview_panel/fallback_renderer.rs` → `render_fallback()`
  - `src/ui/icon_loader/file_icons.rs` → `get_or_load_icon_sized()`
- Problema objetivo:
  - O preview panel chama `get_or_load_icon_sized(..., IconSize::Jumbo, ..., true)`.
  - Com `allow_blocking = true`, o loader pode cair em `extract_file_icon_by_path()`, `extract_shell_icon()` ou `get_file_type_icon()` de forma síncrona.
  - O próprio código reconhece que misses frios podem ser lentos e gera logs de caminho lento.
- Por que isso afeta estabilidade/performance no Windows:
  - Shell icon extraction em Windows pode consultar COM, registry, ProgID, shell namespace e até caminhos virtuais. Em UI immediate-mode isso vira frame stall direto.
- Cenário de reprodução:
  - Selecionar rapidamente executáveis, atalhos, arquivos dentro de archive path, extensões frias ou itens sem thumbnail real.
- Impacto provável:
  - congelamentos de 100-500 ms por seleção,
  - sensação de travamento do app,
  - latência alta ao navegar/selecionar itens no painel.
- Recomendação prática:
  - Nunca permitir lookup bloqueante no render path do preview.
  - Mostrar ícone genérico ou o último ícone conhecido e enfileirar a variante Jumbo de forma assíncrona.

### 4. Rasterização de SVG no image viewer continua síncrona, sem timeout e com teto alto

- Severidade: Alta
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/image_viewer/loader.rs` → `decode_svg_frame()` / `decode_svg_bytes()`
- Problema objetivo:
  - O caminho atual ainda parseia e renderiza SVG de forma síncrona.
  - O teto de rasterização está em `8192`, e não há timeout/cancelamento em torno de `usvg::Tree::from_data()` + `resvg::render()`.
- Por que isso afeta estabilidade/performance no Windows:
  - SVGs patológicos podem consumir CPU por muito tempo e forçar alocações grandes. Um raster de 8192×8192 RGBA sozinho já encosta em ~256 MiB, sem contar estruturas intermediárias e upload de textura.
  - Como isso acontece no viewer dedicado, a janela pode ficar aparentemente travada e ainda pressionar RAM/VRAM do sistema.
- Cenário de reprodução:
  - Abrir SVG com `viewBox` enorme, path complexa, filtros pesados ou simplesmente um SVG muito grande.
- Impacto provável:
  - congelamento do image viewer,
  - pico de RAM/CPU,
  - upload de textura grande demais,
  - degradação perceptível em máquinas com pouca memória/VRAM.
- Recomendação prática:
  - Renderizar SVG em worker isolado com `recv_timeout()`.
  - Reduzir o teto efetivo para 4096.
  - Rejeitar cedo SVGs com dimensão intrínseca claramente abusiva.

### 5. Viewer de imagem ainda reabre sequência e primeiro frame de forma síncrona

- Severidade: Média
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/image_viewer/mod.rs` → `run_standalone()`
  - `src/image_viewer/indexer.rs` → `build_sequence()`
  - `src/image_viewer/app/mod.rs` → `build_initial_cache()` / `open_requested_path()`
- Problema objetivo:
  - O startup do viewer e o caminho de “open request” recebido por pipe ainda fazem `build_sequence()` e `decode_full_frame_with_priority()` de forma síncrona.
  - Isso inclui `read_dir()`, ordenação natural e decode do frame inicial antes do viewer estabilizar o novo estado.
- Por que isso afeta estabilidade/performance no Windows:
  - Diretórios grandes, storage frio, imagens muito grandes ou formatos problemáticos tornam a troca de imagem e até a abertura inicial lentas e com sensação de travamento.
- Cenário de reprodução:
  - Abrir imagem em pasta grande.
  - Enviar novo caminho para uma instância já aberta do viewer.
- Impacto provável:
  - viewer demora a abrir,
  - nova imagem demora para assumir foco,
  - thread de UI do viewer fica bloqueada por I/O/decode.
- Recomendação prática:
  - Mostrar a janela imediatamente e migrar `build_sequence()` + first-frame decode para pipeline assíncrono com placeholder/progresso.

## Riscos prováveis

### 6. `folder-load-pipeline` cria um thread novo por load sem cancelamento real do trabalho antigo

- Severidade: Alta
- Confiança: Média-Alta
- Arquivos / funções afetados:
  - `src/app/operations/folder_loading/mod.rs` → `load_folder()`
  - `src/app/operations/folder_loading/load_pipeline.rs` → `start_folder_load_pipeline()`
  - `src/app/operations/navigation/mod.rs` → múltiplos caminhos que chamam `load_folder(false)`
  - `src/app/operations/folder_loading/load_pipeline/fast_paths.rs`
  - `src/app/operations/folder_loading/load_pipeline/tier3_fallback.rs`
- Problema objetivo:
  - Cada reload/navegação dispara um novo thread de carregamento.
  - O cancelamento é lógico por “generation”, mas os threads antigos continuam até encontrar checkpoints de geração ou até retornarem de chamadas bloqueantes de I/O/Win32.
- Por que isso afeta estabilidade/performance no Windows:
  - Em HDD, OneDrive, shell folders e caminhos lentos, chamadas como `metadata()`, `FindFirstFileW`, `list_shell_folder()`, `read_directory_fast()` e enumeração protegida ainda podem bloquear.
  - Navegação rápida e bursts de watcher podem empilhar trabalho obsoleto e competir com o load atual.
- Cenário de reprodução:
  - navegar muito rápido por várias pastas,
  - receber múltiplos reloads de watcher/file-op,
  - abrir pastas lentas seguidas em storage frio.
- Impacto provável:
  - aumento de threads vivos,
  - I/O redundante,
  - piora da latência de navegação,
  - consumo extra de CPU/disco.
- Recomendação prática:
  - Substituir o modelo “spawn por load” por um worker persistente com fila única e token de cancelamento.

### 7. Watchdog do IPC usa raw handle em thread separada e compete com o teardown do handler

- Severidade: Média
- Confiança: Média
- Arquivos / funções afetados:
  - `crates/mtt-search-service/src/ipc_server/mod.rs` → watchdog criado dentro de `run_ipc_server()`
- Problema objetivo:
  - O watchdog guarda o valor cru do handle (`pipe_raw`) e chama `DisconnectNamedPipe()` por timeout.
  - O handler principal também faz `FlushFileBuffers`, `DisconnectNamedPipe` e `CloseHandle` no mesmo pipe, sem coordenação de ownership entre as duas threads.
- Por que isso afeta estabilidade/performance no Windows:
  - Embora frequentemente resulte apenas em erro, esse padrão é frágil em Windows porque handle fechado pode ser reutilizado, e double-disconnect em timing ruim vira fonte de comportamento errático e bugs difíceis de reproduzir.
- Cenário de reprodução:
  - cliente expira perto do instante em que o handler termina,
  - cliente cai naturalmente pouco antes do watchdog disparar.
- Impacto provável:
  - desconexões espúrias,
  - erros ruidosos em IPC,
  - instabilidade rara mas real sob carga.
- Recomendação prática:
  - Fazer o enforcement de timeout no mesmo thread que owns o pipe, ou usar overlapped I/O com deadline explícita sem thread watchdog separada.

### 8. Pipe do image viewer não tem DACL explícita e aceita pedidos locais sem autenticação

- Severidade: Média
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/image_viewer/ipc.rs` → `create_pipe()` / `start_open_request_server()`
  - `src/image_viewer/app/mod.rs` → `open_requested_path()`
- Problema objetivo:
  - O pipe rejeita clientes remotos, mas usa `None` em `SECURITY_ATTRIBUTES`, ao contrário do search-service, que implementa ACL explícita.
  - Qualquer processo local pode alimentar caminhos válidos em loop e provocar rebuilds de sequência/decode.
- Por que isso afeta estabilidade/performance no Windows:
  - Um processo local não privilegiado consegue forçar churn de I/O, decode e foco da janela do viewer.
- Cenário de reprodução:
  - processo local enviando `open_request` repetidos para o pipe.
- Impacto provável:
  - DoS local do viewer,
  - churn de disco/CPU,
  - trust boundary fraca.
- Recomendação prática:
  - Aplicar ACL semelhante à do search-service e validar o payload antes de reconstruir sequência/estado.

### 9. Worker do PDF reabre documento para cada operação e não tem timeout de render/open

- Severidade: Média
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/pdf_viewer/renderer.rs` → `with_document()` / `render_page()` / `page_text_*()`
  - `src/pdf_viewer/render_worker.rs` → `worker_loop()`
- Problema objetivo:
  - Toda operação reabre o documento via Pdfium.
  - Não há timeout nem watchdog para `load_pdf_from_file()` ou `page.render()`.
- Por que isso afeta estabilidade/performance no Windows:
  - Em PDF grande/corrompido ou com file hooks lentos, um único worker dedicado pode ficar preso por tempo indefinido e deixar o viewer sem progresso visual.
- Cenário de reprodução:
  - zoom/scroll rápido em PDF pesado,
  - abrir PDF danificado,
  - interação com AV/storage lento.
- Impacto provável:
  - blank pages temporárias ou persistentes,
  - alta latência de render,
  - churn extra de handle/I/O.
- Recomendação prática:
  - manter documento vivo no worker, reutilizar handle e adicionar timeout/monitoramento de operação.

## Melhorias de robustez recomendadas

### 10. Consolidar uma estratégia única de shutdown

- Severidade: Alta
- Confiança: Alta
- Causa raiz relacionada: cleanup abrupto e falta de ownership claro do encerramento
- Recomendação de design:
  - Centralizar o shutdown em um state machine explícito:
    - sinalizar cancelamento,
    - desconectar senders,
    - aguardar joins bounded,
    - registrar o que não respondeu,
    - só então aplicar hard-kill como fallback excepcional.

### 11. Separar “snapshot de busca” de “autorização no filesystem”

- Severidade: Alta
- Confiança: Alta
- Causa raiz relacionada: lock hold excessivo + syscalls lentas no caminho crítico
- Recomendação de design:
  - Materializar candidatos em snapshot curto sob lock.
  - Resolver path/autorização fora do lock.
  - Considerar índice imutável por época ou por volume para buscas concorrentes.

### 12. Transformar cargas pesadas de UI em pipelines assíncronos reais

- Severidade: Média
- Confiança: Alta
- Causa raiz relacionada: trabalho síncrono ainda infiltrado em render/startup
- Recomendação de design:
  - mover jumbo icons do preview panel, `build_sequence()`, first-frame decode do image viewer e SVG render para workers dedicados,
  - manter placeholders/stale textures até o resultado novo chegar,
  - nunca deixar shell/registry/decode pesado no caminho do frame.

## Otimizações de performance

### 13. Pooling de documento Pdfium no worker do PDF

- Severidade: Média
- Confiança: Alta
- Oportunidade:
  - `PdfRenderer` hoje reabre o documento em cada operação. Reutilizar o documento dentro do worker reduziria I/O, custo de open/bind e latência de zoom/scroll.

### 14. Fila única/cancelável para folder loading

- Severidade: Média
- Confiança: Alta
- Oportunidade:
  - substituir `spawn` por navegação por um executor único reduz churn de thread e elimina trabalho obsoleto em storage lento.

### 15. Pré-carregamento assíncrono de ícones Jumbo do preview

- Severidade: Média
- Confiança: Alta
- Oportunidade:
  - hoje a aplicação já possui infraestrutura assíncrona de ícones; falta estender isso ao caminho Jumbo do preview panel para remover o último stall síncrono perceptível.

## Risco a monitorar

### 16. `NameArena` não entra mais em panic, mas o scanner fallback ainda pode sair com índice parcial “válido”

- Severidade: Baixa
- Confiança: Média
- Arquivos / funções afetados:
  - `crates/mtt-search-service/src/name_arena.rs`
  - `crates/mtt-search-service/src/fs_walker.rs` → `scan_volume()`
- Observação:
  - `NameArena::insert()` agora retorna `None` ao invés de panic, o que é correto.
  - Porém, `scan_volume()` apenas dá `break` no loop interno quando a arena enche; o scanner continua o restante da fila e pode concluir com índice parcial sem sinalização forte de erro.
- Motivo para monitorar:
  - É improvável em uso normal, mas em volumes gigantescos/non-USN o comportamento final ainda não é “fail closed”.
- Recomendação:
  - propagar erro fatal de arena cheia para o chamador e marcar o índice como `Error`, não `Ready` parcial.

### 17. Renderização de SVG de ícones embutidos permanece síncrona, mas o risco é pequeno pelo tamanho e cache

- Severidade: Baixa
- Confiança: Alta
- Arquivos / funções afetados:
  - `src/ui/svg_icons.rs`
- Observação:
  - O render continua síncrono, mas trabalha sobre assets embutidos, tamanhos pequenos e cache LRU, então o risco atual é principalmente de microstutter em misses, não de exaustão séria.

## Itens verificados e não reabertos no HEAD atual

Os pontos abaixo mostraram mitigação suficiente no código atual e, por isso, não foram reabertos como finding ativo principal nesta auditoria:

- `parking_lot` já substitui os locks do search-service, removendo o risco anterior de poisoning clássico.
- O worker do PDF já usa canais bounded.
- O image viewer já limita workers e janelas de prefetch.
- O drive watcher está opt-in e não é mais caminho padrão.
- O protocolo do search-service já valida estrutura básica de request/response e impõe limite de payload no cliente.

## Prioridade recomendada de correção

1. Remover o `TerminateProcess` do caminho normal de saída e usar shutdown em duas fases.
2. Tirar autorização/`CreateFileW` de dentro do `indices.read()` no search-service.
3. Remover `allow_blocking = true` do preview panel e migrar Jumbo icons para async.
4. Re-hardenizar o decode de SVG do image viewer com timeout + teto menor.
5. Trocar `spawn` por load por um loader cancelável/serializado.
6. Fechar o pipe do image viewer com DACL explícita.
7. Reusar documento Pdfium no worker e adicionar timeout de operação.
