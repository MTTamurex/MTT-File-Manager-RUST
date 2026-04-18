# Auditoria Técnica — MTT File Manager (Rust + Windows)

> Auditoria baseada **exclusivamente no código fonte**. Documentação, comentários e README não foram tratados como fonte de verdade.

**Data:** 2026-04-18  
**Escopo:** `src/`, `crates/mtt-search-service/`, `crates/mtt-search-protocol/`, com foco em `unsafe`, FFI/Win32, concorrência, I/O, performance, arquitetura e tratamento de erros.

---

## 1. Resumo Geral

**Estado do projeto.** Codebase grande, com integração profunda em Win32, NTFS/MFT, USN Journal, `ReadDirectoryChangesW`, COM/Shell, named pipes e múltiplos workers. A separação em `domain / application / infrastructure / app / ui / workers` existe, mas há vazamento de camada em pontos críticos.

**Principais riscos técnicos identificados:**

1. Vazamentos reais de `HANDLE` em caminhos de erro do `drive_watcher` e `mft_reader`.
2. Buffer de `ReadDirectoryChangesW` pequeno demais para diretórios com alto churn, com perda silenciosa de eventos.
3. Parsing binário não confiável com `unwrap()` em massa no `mft_reader`, capaz de derrubar o search service.
4. Locks de write segurados por tempo demais em indexação/busca, bloqueando consultas IPC.
5. `catch_unwind` com resultado descartado em workers, causando operações travadas sem resposta para a UI.
6. Tipos opacos Win32 tratados como inteiros crus (`HDEVNOTIFY` em `AtomicUsize`), o que degrada segurança e manutenção.
7. Uso inconsistente de COM apartment model e lifecycle em workers.
8. Estruturas de coalescing/caching sem limite rígido em cenários adversariais.
9. Arquivos excessivamente grandes concentrando múltiplas responsabilidades.
10. Tratamento de erro inconsistente, com perda frequente de contexto Win32 e de diagnóstico.

---

## 2. Problemas Críticos

### C1. Buffer de `ReadDirectoryChangesW` insuficiente e sem tratamento de overflow
- **Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs`
- **Impacto:** **Crítico**
- **Cenário:** diretórios com OneDrive/Dropbox, builds grandes, extrações massivas, árvores com milhares de mudanças por segundo.
- **Problema:** `BUFFER_SIZE = 65536` é insuficiente para bursts reais. Quando o buffer satura, o Windows pode truncar notificações sem erro fatal, levando a perda de eventos e cache inconsistente.
- **Causa raiz:** escolha de buffer conservadora demais e ausência de tratamento explícito para saturação.
- **Correção recomendada:** elevar para 256–512 KB onde aplicável e, ao detectar saturação/truncamento, invalidar o prefixo inteiro em vez de confiar no batch parcial.

### C2. Leak de `HANDLE` do evento no drive watcher
- **Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs`
- **Impacto:** **Alto**
- **Cenário:** watcher reiniciado, erro de abertura de handle, saída de thread, falhas intermitentes de volume.
- **Problema:** `CreateEventW` aloca um kernel handle que não é fechado de forma garantida em todos os caminhos.
- **Causa raiz:** gerenciamento manual de handles sem RAII consistente.
- **Correção recomendada:** encapsular `HANDLE` em wrapper com `Drop` chamando `CloseHandle`.

### C3. Leak de handle de volume em `mft_reader`
- **Arquivo:** `crates/mtt-search-service/src/mft_reader.rs`
- **Impacto:** **Alto**
- **Cenário:** erro em `GetFileSizeEx`, `DeviceIoControl`, leitura parcial de volume, volume corrompido.
- **Problema:** o volume é aberto com `CreateFileW`, mas certos retornos antecipados não garantem `CloseHandle`.
- **Causa raiz:** early returns em fluxo Win32 sem cleanup centralizado.
- **Correção recomendada:** usar guard RAII para todo handle aberto em `mft_reader`.

### C4. `unwrap()` em parsing binário não confiável do NTFS/MFT
- **Arquivo:** `crates/mtt-search-service/src/mft_reader.rs`
- **Impacto:** **Crítico**
- **Cenário:** volume com boot sector inconsistente, disco removível degradado, setor parcialmente lido, dados truncados.
- **Problema:** há diversas leituras do tipo `buffer[a..b].try_into().unwrap()` sobre bytes externos e não confiáveis.
- **Causa raiz:** parser assume layout íntegro sem validar tamanho mínimo antes de cada acesso.
- **Correção recomendada:** substituir `unwrap()` por validação explícita de faixa e retorno `Result` com contexto (`offset`, `expected_len`, volume).

### C5. `catch_unwind` inócuo em worker de file operations
- **Arquivo:** `src/workers/file_operation_worker.rs`
- **Impacto:** **Alto**
- **Cenário:** panic dentro de handler de delete/move/restore.
- **Problema:** o resultado de `catch_unwind` é ignorado. Se o handler panica, a UI pode nunca receber resposta.
- **Causa raiz:** tratamento incompleto de falha em worker assíncrono.
- **Correção recomendada:** logar o panic e enviar `FileOperationResult::Error` no canal de retorno.

### C6. `h.join().unwrap()` em worker de thumbnails
- **Arquivo:** `src/workers/thumbnail/worker.rs`
- **Impacto:** **Crítico**
- **Cenário:** decoder/codec panica ao processar entrada inválida.
- **Problema:** panic da thread secundária escala para quem faz `join()`, derrubando o processo.
- **Causa raiz:** `unwrap()` em fronteira de thread.
- **Correção recomendada:** tratar `join()` com `if let Err(...)` e converter para log/telemetria.

### C7. `HDEVNOTIFY` armazenado como `AtomicUsize`
- **Arquivo:** `src/infrastructure/windows/device_change.rs`
- **Impacto:** **Alto**
- **Cenário:** desregistro de device notifications, shutdown, refator futura, corrupção de valor intermediário.
- **Problema:** handle opaco da Win32 é rebaixado a inteiro cru, perdendo semântica e validação de tipo.
- **Causa raiz:** workaround para `Send + Sync` feito no nível errado.
- **Correção recomendada:** armazenar em wrapper tipado, com lifecycle encapsulado por `Mutex`/`OnceLock`.

---

## 3. Problemas de Segurança (Rust / Unsafe / FFI)

### S1. Layout serializado dependente de layout in-memory sem contrato formal
- **Arquivo:** `crates/mtt-search-service/src/index_db/binary.rs`
- **Impacto:** **Alto**
- **Cenário:** mudança futura em `Header`/`FileRecord`, padding alterado pelo compilador, build cross-target.
- **Problema:** conversões entre struct e bytes usam casts e `read_unaligned`, mas não há garantias suficientes de layout.
- **Causa raiz:** serialização manual baseada em memória bruta.
- **Correção recomendada:** usar `#[repr(C)]`/`#[repr(C, packed)]` quando apropriado, mais asserts estáticos de `size_of`, ou serialização campo a campo.

### S2. `GlobalLock` sem validação de ponteiro nulo
- **Arquivo:** `src/infrastructure/windows_clipboard.rs`
- **Impacto:** **Médio**
- **Cenário:** falha de alocação/lock do clipboard global memory.
- **Problema:** o retorno pode ser `NULL`, mas o ponteiro é usado como se fosse válido.
- **Causa raiz:** pressuposto de sucesso em API Win32 que explicitamente pode falhar.
- **Correção recomendada:** checar `is_null()` antes de copiar para o buffer.

### S3. `transmute` de ponteiro de função retornado por `GetProcAddress`
- **Arquivo:** `src/infrastructure/ntfs_reader.rs`
- **Impacto:** **Médio**
- **Cenário:** divergência entre assinatura declarada localmente e ABI real exportada pela DLL.
- **Problema:** `transmute` de function pointer é frágil e vira UB se a assinatura estiver errada.
- **Causa raiz:** binding manual quando já há crates com assinatura correta.
- **Correção recomendada:** preferir bindings tipados do `windows-sys`/`windows` e evitar `GetProcAddress` manual.

### S4. Impersonation guard sem distinção entre sucesso real e sucesso lógico
- **Arquivo:** `crates/mtt-search-service/src/ipc_authorization.rs`
- **Impacto:** **Médio**
- **Cenário:** nested impersonation ou caminhos em que a thread já está sob contexto alterado.
- **Problema:** `RevertToSelf()` pode ser chamado em escopo incorreto se o guard sempre marcar `active = true`.
- **Causa raiz:** ausência de modelagem explícita do estado retornado pela API.
- **Correção recomendada:** guardar estado mais preciso do impersonation guard e só reverter quando a chamada efetivamente tiver iniciado uma nova impersonação.

### S5. ACL/SID montados à mão em buffer cru
- **Arquivo:** `crates/mtt-search-service/src/ipc_server/pipe_io.rs`
- **Impacto:** **Médio**
- **Cenário:** mudança futura em SID/ACE, erro de manutenção, revisão de segurança do pipe.
- **Problema:** tamanhos e offsets são calculados manualmente, o que é frágil e fácil de quebrar.
- **Causa raiz:** construção artesanal de descritores de segurança em vez de APIs auxiliares.
- **Correção recomendada:** migrar para funções Win32 próprias para ACL/SID.

### S6. Uso inconsistente de COM apartment model
- **Arquivo:** `src/workers/file_operation_worker.rs` e outros workers COM-related
- **Impacto:** **Alto**
- **Cenário:** chamada COM em thread diferente daquela que inicializou STA, ou refator futura que insira `spawn` extra.
- **Problema:** o projeto depende de garantias implícitas de afinidade de thread.
- **Causa raiz:** falta de abstração única para escopo COM.
- **Correção recomendada:** criar RAII `ComScope` por operação crítica, com `CoInitializeEx` e `CoUninitialize` no mesmo escopo e mesma thread.

---

## 4. Problemas de Performance

### P1. Dupla chamada a `metadata()` em caminho quente de vídeo
- **Arquivo:** `src/infrastructure/windows/metadata/video.rs`
- **Impacto:** **Crítico**
- **Cenário:** pastas com milhares de vídeos, geração de thumbnails/metadata em lote.
- **Problema:** o código busca tamanho do arquivo via `metadata()` mais de uma vez na mesma composição de metadados.
- **Causa raiz:** falta de passagem de `file_size` já conhecido no caller.
- **Correção recomendada:** passar `file_size` como argumento e cortar syscall redundante.

### P2. Conversões `OsString -> String` repetidas em loops de UI
- **Arquivos:** `src/image_viewer/mod.rs`, `src/workers/thumbnail/progress.rs` e outros
- **Impacto:** **Alto**
- **Cenário:** listas grandes redesenhadas por frame, navegação rápida, grids com centenas/milhares de itens.
- **Problema:** `to_string_lossy().to_string()` aparece repetidamente em hot path.
- **Causa raiz:** falta de reutilização do nome já materializado em `FileEntry`.
- **Correção recomendada:** usar `&entry.name` ou cachear o nome convertido uma vez.

### P3. Evicção de cache de imagens via `collect + sort`
- **Arquivo:** `src/image_viewer/cache.rs`
- **Impacto:** **Médio**
- **Cenário:** navegação sequencial por imagens grandes, pressão constante de memória.
- **Problema:** gera `Vec<usize>` e ordena tudo para remover alguns poucos itens.
- **Causa raiz:** algoritmo simples demais para hot path de eviction.
- **Correção recomendada:** usar `BinaryHeap`, ou ao menos `max_by_key` incremental.

### P4. Lock de write global grande demais na indexação USN
- **Arquivo:** `crates/mtt-search-service/src/volume_indexers/usn.rs`
- **Impacto:** **Alto**
- **Cenário:** index rebuild parcial enquanto o usuário digita na busca global.
- **Problema:** consultas IPC ficam bloqueadas enquanto o lock de write é mantido durante lotes longos.
- **Causa raiz:** granularidade de lock excessivamente ampla.
- **Correção recomendada:** lock por volume, batch-commit menor, ou swap atômico de estrutura pronta.

### P5. Clone integral da arena de nomes para lowercase
- **Arquivo:** `crates/mtt-search-service/src/name_arena.rs`
- **Impacto:** **Alto**
- **Cenário:** índice com centenas de milhares ou milhões de arquivos.
- **Problema:** `self.lowered = self.buf.clone()` duplica um buffer grande logo no início.
- **Causa raiz:** estratégia eager e monolítica de normalização.
- **Correção recomendada:** lowercase lazy/incremental, ou buckets normalizados sob demanda.

### P6. EXIF lido com múltiplas buscas por tag por imagem
- **Arquivo:** `src/infrastructure/windows/metadata/image.rs`
- **Impacto:** **Alto**
- **Cenário:** diretórios grandes com milhares de imagens.
- **Problema:** faz diversas buscas individuais por tag depois da leitura do EXIF.
- **Causa raiz:** falta de materialização compacta do resultado da leitura.
- **Correção recomendada:** varrer os fields uma única vez e montar uma struct cache.

### P7. Polling de retry com intervalos fixos curtos
- **Arquivos:** `src/app/init_workers/filesystem_workers.rs`, `src/image_viewer/ipc.rs`
- **Impacto:** **Médio**
- **Cenário:** startup lenta do serviço, indisponibilidade temporária de IPC.
- **Problema:** loops com `sleep(50ms/60ms)` acumulam wakeups e context switches desnecessários.
- **Causa raiz:** backoff simplista.
- **Correção recomendada:** backoff exponencial ou espera por evento.

### P8. `pending_revalidation` sem prune previsível
- **Arquivo:** `src/app/folder_size_state.rs`
- **Impacto:** **Médio**
- **Cenário:** navegação extensa por muitos diretórios, sem retorno aos anteriores.
- **Problema:** o mapa acumula entradas antigas e aumenta custo/uso de memória sem necessidade.
- **Causa raiz:** ausência de rotina de limpeza.
- **Correção recomendada:** `retain()` periódico baseado em deadline expirado.

### P9. Invalidação de filhos no `DirectoryCache` é linear sobre toda a estrutura
- **Arquivo:** `src/infrastructure/directory_cache.rs`
- **Impacto:** **Médio**
- **Cenário:** remoção/renomeação de subárvores grandes.
- **Problema:** varre o cache inteiro procurando prefixos, clonando `PathBuf` para remoção.
- **Causa raiz:** estrutura não preparada para prefix queries.
- **Correção recomendada:** `BTreeMap`/trie por prefixo, ou outra indexação auxiliar.

---

## 5. Problemas na Integração com Windows API

### W1. `OVERLAPPED` reaproveitado sem reset completo
- **Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs`
- **Impacto:** **Médio**
- **Cenário:** múltiplas iterações assíncronas no watcher.
- **Problema:** o struct é reaproveitado, mas a limpeza do estado implícito não é robusta.
- **Causa raiz:** otimização manual sem encapsulamento.
- **Correção recomendada:** rezerar o `OVERLAPPED` antes de cada uso, preservando apenas `hEvent`.

### W2. Falta de `GetLastError`/contexto após falha de `DeviceIoControl`
- **Arquivos:** `crates/mtt-search-service/src/mft_reader.rs`, `crates/mtt-search-service/src/usn_journal.rs`
- **Impacto:** **Alto**
- **Cenário:** `ERROR_ACCESS_DENIED`, `ERROR_INVALID_PARAMETER`, mídia removida, journal indisponível.
- **Problema:** a falha é tratada genericamente, sem erro Win32 concreto.
- **Causa raiz:** logging insuficiente nas fronteiras FFI.
- **Correção recomendada:** capturar e logar `GetLastError().0` em todos os paths `is_err()`.

### W3. UI chamando Win32 diretamente
- **Arquivo:** `src/ui/app/lifecycle.rs`
- **Impacto:** **Médio**
- **Cenário:** snapshot de threads/processo durante lifecycle.
- **Problema:** mistura rendering/lifecycle UI com syscalls Win32 de baixo nível.
- **Causa raiz:** fronteira arquitetural vazando.
- **Correção recomendada:** mover a lógica para `infrastructure::windows::*`.

### W4. Construção manual de ACL/SID do named pipe
- **Arquivo:** `crates/mtt-search-service/src/ipc_server/pipe_io.rs`
- **Impacto:** **Médio**
- **Cenário:** manutenção futura do pipe security descriptor.
- **Problema:** bytes montados manualmente são frágeis e difíceis de auditar.
- **Causa raiz:** ausência de uso das helpers da própria Win32.
- **Correção recomendada:** trocar por APIs canônicas de segurança do Windows.

### W5. `SetDefaultDllDirectories` com resultado descartado
- **Arquivo:** `src/main.rs`
- **Impacto:** **Alto**
- **Cenário:** hardening falha silenciosamente no startup.
- **Problema:** se a chamada falha, o processo continua sem visibilidade de risco.
- **Causa raiz:** descarte explícito do retorno.
- **Correção recomendada:** logar falha e, se apropriado, endurecer política de fallback.

---

## 6. Problemas de Concorrência

### Co1. Estrutura de coalescing sem limite efetivo antes do insert
- **Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs`
- **Impacto:** **Crítico**
- **Cenário:** storms de eventos em cloud sync, builds, extrações.
- **Problema:** `HashSet` pode crescer demais antes de flush/controle efetivo.
- **Causa raiz:** checagem do tamanho ocorre tarde demais.
- **Correção recomendada:** flush/cap antes de inserir novos eventos.

### Co2. Recuperação cega de mutex envenenado
- **Arquivos:** `src/infrastructure/directory_cache.rs`, `src/infrastructure/directory_dirty_registry.rs` e outros
- **Impacto:** **Alto**
- **Cenário:** panic sob lock durante atualização de cache.
- **Problema:** `unwrap_or_else(|e| e.into_inner())` mantém o processo rodando, mas pode expor estrutura corrompida.
- **Causa raiz:** recuperação genérica sem reestabelecer invariantes.
- **Correção recomendada:** usar `parking_lot::Mutex` e, quando houver falha séria, limpar/reconstruir o estado.

### Co3. `notify_one()` com lock ainda segurado
- **Arquivo:** `src/workers/thumbnail/queue.rs`
- **Impacto:** **Médio**
- **Cenário:** alta contenção na fila de thumbnails.
- **Problema:** gera inversão de prioridade e wakeup menos eficiente.
- **Causa raiz:** ordem errada de unlock/notify.
- **Correção recomendada:** dropar o guard antes do `notify_one()`.

### Co4. Flags atômicos podem ficar presos em `true` se `spawn()` falhar
- **Arquivos:** `crates/mtt-search-service/src/ipc_server/handler.rs`, `src/workers/global_search_worker.rs`
- **Impacto:** **Alto**
- **Cenário:** resource exhaustion ou falha de criação de thread.
- **Problema:** o flag de in-flight/warming é setado antes do `spawn`, e pode nunca ser resetado se o spawn falhar.
- **Causa raiz:** falta de rollback em erro de thread creation.
- **Correção recomendada:** resetar explicitamente o flag no branch de erro do `spawn()`.

### Co5. Inicialização concorrente do PDFium
- **Arquivo:** `src/pdf_viewer/renderer.rs`
- **Impacto:** **Alto**
- **Cenário:** duas threads entram em `pdfium()` antes da primeira concluir o bind.
- **Problema:** múltiplas tentativas paralelas de bind/load da DLL.
- **Causa raiz:** uso de `OnceCell` sem inicialização protegida por `get_or_try_init`/lock.
- **Correção recomendada:** serializar a inicialização.

### Co6. Ordem atômica excessiva e assimétrica no GC worker
- **Arquivo:** `src/app/init_workers/background_jobs.rs`
- **Impacto:** **Médio**
- **Cenário:** shutdown e polling em arquiteturas mais fracas que x86.
- **Problema:** o uso atual não é o mais claro nem o mais eficiente para uma flag binária.
- **Causa raiz:** modelo de sincronização superespecificado em alguns pontos e subdocumentado em outros.
- **Correção recomendada:** simplificar com ordem adequada ao caso real, ou trocar por mecanismo explícito de wakeup.

### Co7. Estado lido/escrito de forma inconsistente no drive watcher
- **Arquivo:** `src/infrastructure/drive_watcher.rs`
- **Impacto:** **Médio**
- **Cenário:** atualização simultânea de prefixo observado durante processamento de eventos.
- **Problema:** o mesmo estado é acessado por caminhos com sincronização heterogênea.
- **Causa raiz:** falta de abstração única para o prefixo observado.
- **Correção recomendada:** `ArcSwap`, `RwLock` ou sincronização única consistente.

---

## 7. Problemas de Arquitetura

### A1. Arquivos excessivamente grandes e com responsabilidades múltiplas

**Top arquivos grandes identificados:**

1. `crates/mtt-search-service/src/mft_reader.rs` — ~1150 linhas — parser MFT, atributos e bootstrap misturados.
2. `src/app/init_bootstrap.rs` — ~500+ linhas — setup de estado, workers, DB e bootstrap combinados.
3. `src/app/state/sidebar_tree_state.rs` — ~420+ linhas — árvore, navegação, drag/drop misturados.
4. `src/application/file_operations.rs` — ~370 linhas — business logic + COM/Shell + clipboard/file ops em conjunto.
5. `src/infrastructure/global_search.rs` — ~410 linhas — IPC client e validação em um único bloco.

**Impacto:** **Alto**

**Correção recomendada:** quebrar por fronteira funcional, especialmente `mft_reader.rs`, `init_bootstrap.rs` e `sidebar_tree_state.rs`.

### A2. Vazamento de camada entre UI e infraestrutura
- **Arquivo:** `src/ui/app/lifecycle.rs`
- **Impacto:** **Médio**
- **Problema:** a UI executa syscalls diretamente.
- **Correção recomendada:** mover isso para `infrastructure::windows::*` e expor API mais limpa para a UI.

### A3. Estado duplicado e invalidação espalhada
- **Arquivos:** `src/app/folder_size_state.rs`, `src/app/cache_state.rs`, `src/app/global_search_state.rs`, helpers de message handler
- **Impacto:** **Alto**
- **Problema:** múltiplos caches representam aspectos próximos do mesmo dado, e a invalidação depende de muitos `pop()` e updates distribuídos.
- **Correção recomendada:** centralizar eventos de invalidação e inscrição de caches em um barramento interno.

### A4. Fronteira confusa entre `workers/` e `app/init_workers/`
- **Impacto:** **Médio**
- **Problema:** lifecycle de worker não está concentrado num único registry/owner.
- **Correção recomendada:** um módulo central de registro, start, stop e join de workers.

### A5. Tipos de erro inconsistentes
- **Arquivos:** `src/domain/errors.rs`, `src/infrastructure/global_search.rs`, `src/application/file_operations.rs`
- **Impacto:** **Alto**
- **Problema:** coexistem `AppError`, `String`, aliases locais e perda de contexto entre módulos.
- **Correção recomendada:** criar erros tipados por subsistema e padronizar conversão.

---

## 8. Melhorias Recomendadas

1. Criar `OwnedHandle` e adotar RAII para todo `HANDLE` Win32 do projeto.
2. Criar `ComScope`/`ComGuard` único, usado por todas as operações COM.
3. Eliminar `Result<T, String>` em fronteiras públicas e padronizar erros tipados por domínio.
4. Refatorar `mft_reader.rs` em módulos menores: geometria, atributos, records, helpers Win32.
5. Refatorar `init_bootstrap.rs` em setup de canais, setup de DB, setup de workers e state wiring.
6. Mover qualquer syscall Win32 da UI para `infrastructure::windows::*`.
7. Reduzir escopo de locks em indexação e busca.
8. Reforçar limites rígidos em estruturas de batching, coalescing e caches pendentes.
9. Substituir construções manuais de ACL/SID e `GetProcAddress` manual por bindings Win32 seguros.
10. Instrumentar com profiling real os hot paths de search, metadata e thumbnailing.

---

## 9. Quick Wins (alto impacto / baixo esforço)

1. Aumentar o buffer do `ReadDirectoryChangesW` e invalidar prefixo ao detectar truncamento.
2. Trocar `h.join().unwrap()` por tratamento explícito com log.
3. Processar o retorno de `catch_unwind` em `file_operation_worker` e responder à UI.
4. Passar `file_size` já conhecido para `merge_video_metadata`.
5. Resetar flags in-flight quando `spawn()` falhar.
6. Parar de usar `to_string_lossy().to_string()` em loops de UI quando o nome já existe em cache.
7. Adicionar bounds checks antes de todos os `try_into().unwrap()` em parsing binário do MFT.
8. Logar `GetLastError()` após falhas de `DeviceIoControl`.
9. Aplicar RAII para `h_event` e handles de volume.
10. Mover `notify_one()` para fora do lock nas filas de thumbnail.
11. Fazer prune periódico de `pending_revalidation`.
12. Serializar a inicialização do PDFium com `get_or_try_init` ou lock.

---

## Conclusão

O projeto não sofre de um único erro estrutural catastrófico; ele sofre de **acúmulo de riscos reais em fronteiras Win32/FFI, parsing binário e concorrência**, exatamente onde aplicações Windows nativas costumam quebrar em produção pesada.

Os problemas mais urgentes não são cosméticos nem estilísticos. Eles concentram-se em:

1. `HANDLE` e cleanup incompleto.
2. Parsing binário inseguro com `unwrap()` em dados externos.
3. Locks e workers com falha silenciosa.
4. Hot paths com syscalls e alocações redundantes.
5. Falta de uniformidade em abstrações de erro, COM e Win32 resource management.

Se a meta é endurecer o sistema para cargas reais e cenários extremos do Windows, o caminho correto é: **RAII rigoroso para recursos Win32, eliminação de `unwrap()` em dados externos, refino do modelo de concorrência e redução do custo por evento/arquivo**.