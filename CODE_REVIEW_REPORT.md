# Relatório de Code Review — MTT File Manager (Rust)

- **Data:** 2026-07-17
- **Escopo:** Projeto inteiro (`src/`, `crates/`, `benches/`, `build.rs`)
- **Focos priorizados:** Segurança · Correção/Bugs lógicos · Performance
- **Base analisada:** 434 arquivos Rust · ~114.370 LOC (exceto `target/` e `vendor/`)
- **Observação:** A documentação existente do projeto **não** foi usada como base (considerada desatualizada). As conclusões vêm da leitura direta do código.

> **Metodologia:** varredura por padrões de risco (unsafe, unwrap, spawn de processo, I/O síncrono, casts, locks) + leitura aprofundada dos módulos mais críticos e maiores. Em uma base de ~114k LOC, este relatório prioriza os achados de maior sinal; não é uma auditoria exaustiva linha-a-linha de cada arquivo.

---

## 1. Sumário Executivo

O projeto está, em termos gerais, **bem arquitetado e com forte consciência de segurança**. A separação em camadas (`domain` / `application` / `infrastructure`) é respeitada, o isolamento do serviço de busca em um processo separado com fronteira IPC endurecida é exemplar, e há uso deliberado de aritmética saturante, `catch_unwind` em workers e validação de paths em múltiplas camadas.

Os problemas encontrados concentram-se em três categorias:

1. **Performance na thread de UI** — o achado de maior impacto: I/O de filesystem síncrono dentro do loop de render (pode congelar a UI em paths de rede/OneDrive).
2. **Robustez/Correção** — `unwrap()` em pontos de inicialização, erros de escrita em banco silenciosamente descartados, e casts de dimensões vindas de workers sem validação.
3. **Segurança** — poucos itens reais, principalmente uma injeção de comando via nome de diretório no "abrir terminal como admin".

### Contagem por severidade

| Severidade | Qtd | Categorias |
|---|---|---|
| 🔴 Crítico | 1 | Performance |
| 🟠 Alto | 5 | Segurança(1), Correção(2), Performance(2) |
| 🟡 Médio | 8 | Correção(4), Performance(3), Segurança(1) |
| 🔵 Baixo/Info | 9 | Correção(4), Performance(3), Segurança(2) |

---

## 2. 🔴 Crítico

### PERF-C1 — I/O de filesystem síncrono na thread de UI (busca global com filtro de tags)
**Arquivo:** [`results_panel.rs#L670`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/global_search_overlay/results_panel.rs#L619-L699)

`ensure_tagged_results_for_active_filter` é chamada no topo de `render_results_panel` (a cada frame). Quando há um filtro de tag ativo e a cache-key muda (usuário digita / muda filtro / muda epoch de tags), o código itera até `TAGGED_RESULTS_INJECTION_LIMIT = 2_000` paths e chama **`std::fs::metadata(path)` de forma síncrona** para cada um:

```rust
let Ok(metadata) = std::fs::metadata(path) else { continue; };
```

`std::fs::metadata` no Windows abre um handle de kernel por arquivo. Em paths de rede ou arquivos OneDrive "cloud-only", cada chamada pode **bloquear por segundos**, congelando toda a UI (o egui redesenha por frame). O próprio código em [`list_bridge.rs#L277-L283`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/ui_rendering/list_bridge.rs#L276-L283) documenta que `GetFileAttributesW` foi removido do render loop exatamente por causar esse tipo de freeze — mas aqui o padrão reincide via `std::fs::metadata`.

**Recomendação:** mover a resolução de metadados dos itens injetados para o worker de metadados já existente (canal assíncrono) ou usar `GetFileAttributesExW` (não abre handle, retorna do cache de diretório em microssegundos), como já é feito em [`tag_ops/view.rs#L28-L50`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/tag_ops/view.rs#L28-L50). Nunca fazer I/O de rede/cloud dentro do loop de render.

---

## 3. 🟠 Alto

### SEC-A1 — Injeção de comando PowerShell via nome de diretório (terminal admin)
**Arquivo:** [`menu_handler.rs#L84-L88`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/app/menu_handler.rs#L75-L89)

```rust
let cd_cmd = format!("cd '{}'", dir.display());
elevated_spawn("powershell.exe", &["-NoExit", "-Command", &cd_cmd]);
```

O caminho do diretório é interpolado dentro de um `-Command` do PowerShell **com aspas simples, sem escapar**. Um diretório cujo nome contenha `'` (ex.: `C:\a'; Start-Process calc; '`) quebra a string e injeta comandos arbitrários — **executados com elevação (UAC)**, pois `elevated_spawn` usa o verbo `runas`. Como a operação é "Abrir terminal (admin) aqui", o atacante precisa que o usuário navegue até a pasta e acione o menu, mas o resultado é execução elevada arbitrária.

**Recomendação:** não passar o diretório via `-Command`. Preferir `Set-Location -LiteralPath` com o path passado como argumento separado (não concatenado), ou usar `powershell.exe -NoExit -Command Set-Location -LiteralPath` com o path devidamente escapado (duplicando `'` → `''`). O caminho ideal é usar o parâmetro de working directory do próprio `ShellExecuteExW` (`lpDirectory`) em vez de compor um comando.

### BUG-A1 — `join().unwrap()` em threads de inicialização derruba o app
**Arquivo:** [`init_bootstrap.rs#L212-L251`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/init_bootstrap.rs#L200-L255) e [`#L308`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/init_bootstrap.rs#L308)

```rust
let app_state_raw = app_state_handle.join().unwrap();
let disk_cache_raw = disk_cache_handle.join().unwrap();
// ...
let dir_index_raw = dir_index_handle.join().unwrap();
let icon_cache_inner = icon_cache_handle.join().unwrap();
// ...
let (folder_composer_raw, custom_folder_icon) = folder_composer_handle.join().unwrap();
```

Se **qualquer** thread de bootstrap entrar em pânico (falha inesperada de SQLite, exaustão de recurso, etc.), o `join().unwrap()` propaga o pânico para a thread principal e o aplicativo fecha na inicialização, sem recuperação nem diagnóstico claro. Note que a lógica de fallback (in-memory) já existe logo abaixo — mas ela só é alcançada se o thread retornar `Err`, não se ele entrar em *panic*.

**Recomendação:** tratar o `Result` de `join()` explicitamente (`match`/`unwrap_or_else`) e logar/degradar em vez de propagar o pânico; considerar `catch_unwind` dentro das closures dos threads de init.

### BUG-A2 — Erros de escrita no banco silenciosamente descartados (perda de estado)
**Arquivos:**
- [`preferences.rs#L7-L55`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/app_state_db/preferences.rs#L5-L55) (`let _ = db.execute(...)`, inclusive `COMMIT`)
- [`folder_locks.rs#L58-L63`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/app_state_db/folder_locks.rs#L58-L64) (remoção de lock)
- [`file_entry_cache.rs#L253-L330`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/app_state_db/file_entry_cache.rs#L250-L330) (`let _ = tx.execute` e `let _ = tx.commit()`)

Escritas críticas de estado persistente descartam o `Result`. Em caso de disco cheio, banco bloqueado ou corrompido, a preferência/lock/cache é **perdida silenciosamente**, sem log nem notificação. Para `folder_locks`, o usuário pode acreditar que travou/destravou uma pasta quando a operação falhou.

**Recomendação:** ao menos registrar o erro (`log::warn!`/`error!`). Para operações sensíveis à consistência (locks), propagar/refletir a falha na UI.

### PERF-A1 — `to_lowercase()` por tag a cada frame na sidebar
**Arquivo:** [`sidebar.rs#L415-L416`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/sidebar.rs#L410-L435)

```rust
let mut tags: Vec<&FileTag> = ctx.tag_definitions.values().collect();
tags.sort_by_key(|tag| (tag.position, tag.name.to_lowercase()));
```

Executa `to_lowercase()` (aloca uma `String` nova por tag) **em toda renderização** da seção de tags, mais o `collect` + `sort`. Com muitas tags e a sidebar sempre visível, é alocação e ordenação redundantes a 60 FPS.

**Recomendação:** manter a lista de tags já ordenada em cache (invalidada por epoch de tags) ou pré-computar a chave de ordenação em minúsculas ao carregar/alterar tags.

### PERF-A2 — `sorted_tag_definitions()` clona e ordena todas as tags a cada frame
**Arquivos:** [`global_search_overlay.rs#L957`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/global_search_overlay.rs#L954-L960) → [`tag_ops/view.rs#L102-L106`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/tag_ops/view.rs#L101-L106)

```rust
pub fn sorted_tag_definitions(&self) -> Vec<FileTag> {
    let mut tags: Vec<FileTag> = self.tag_definitions.values().cloned().collect(); // clona todas
    tags.sort_by_key(tag_sort_key);
    tags
}
```

Chamada por frame enquanto o popup de filtro de tags está aberto: **clona todos os `FileTag`**, coleta e ordena. Alocação O(n) desnecessária por frame.

**Recomendação:** cachear o vetor ordenado (invalidado por `tag_assignments_epoch`) e emprestar por referência em vez de clonar.

---

## 4. 🟡 Médio

### BUG-M1 — Casts de dimensões vindas de workers sem validação (risco de OOM/alloc panic)
**Arquivos:** [`global_search_events.rs#L266-L272`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/message_handler/global_search_events.rs#L259-L275) e [`thumbnail_uploads.rs#L960-L970`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/message_handler/thumbnail_uploads.rs#L959-L970)

```rust
egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba)
```

`width`/`height` chegam do pipeline de thumbnail. Se um resultado corrompido trouxer dimensões enormes, o cast cria uma `ColorImage` gigante → panic de alocação ou OOM. Não há checagem de que `width * height * 4 == rgba.len()`.

**Recomendação:** validar `width`/`height` contra limites máximos e conferir `rgba.len() == width*height*4` antes de construir a imagem; descartar resultados inconsistentes.

### BUG-M2 — `compute_texture_cache_target_items`: cast `as i32` pode transbordar
**Arquivo:** [`thumbnail_uploads.rs#L80-L96`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/message_handler/thumbnail_uploads.rs#L55-L97)

```rust
let raw_target = ((visible_base * tab_factor).round() as i32) + backlog_boost + ...;
```

Se `visible_base * tab_factor` exceder `i32::MAX`, o cast `as i32` produz valor indefinido (saturação em float→int), e o `clamp` posterior opera sobre lixo. Improvável em uso normal, mas é um cálculo de capacidade de cache.

**Recomendação:** clampear em `f32` antes do cast, ou usar `saturating` via `.clamp()` já no domínio float.

### BUG-M3 — `window_subclass`: ordering atômico misto + fallback silencioso em mutex envenenado
**Arquivo:** [`window_subclass.rs#L128-L184`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/windows/window_subclass.rs#L105-L196)

`LAYOUT_PHASE` é lido com `Ordering::Relaxed` mas escrito com `SeqCst` (assimetria inconsistente); `try_unfreeze_layout` faz check-then-act (load Relaxed → store SeqCst) sem CAS. Além disso, `freeze_layout`/`get_frozen_sidebar_widths` usam `if let Ok(...)` sobre `std::sync::Mutex`, revertendo silenciosamente para defaults `(200.0, 300.0)` se o mutex estiver envenenado.

**Recomendação:** padronizar orderings (Acquire/Release coerentes) e usar `compare_exchange` na transição de fase; considerar `parking_lot::Mutex` (sem poison) e logar o fallback.

### BUG-M4 — `tag_ops/cache.rs`: `unwrap()` após possível mutação de estado
**Arquivo:** [`tag_ops/cache.rs#L36-L59`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/tag_ops/cache.rs#L36-L60)

```rust
let snapshot = self.dual_panel_inactive_state.as_mut().unwrap();
```

O `unwrap` no ramo `else` depende de `with_inactive_panel` (chamado no ramo `if`) não zerar `dual_panel_inactive_state`. É logicamente seguro hoje, mas frágil a refatorações — vira panic se a invariante quebrar.

**Recomendação:** substituir por `if let Some(...)`/`let ... else return`.

### SEC-M1 — `elevated_spawn`: quoting de argumentos não escapa aspas embutidas
**Arquivo:** [`menu_handler.rs#L45-L57`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/app/menu_handler.rs#L41-L71)

Os argumentos só são envoltos em aspas se contiverem espaço, e aspas internas não são escapadas. Combinado com SEC-A1, reforça o vetor de injeção quando um argumento (como o diretório) contém `"` ou `'`.

**Recomendação:** aplicar quoting/escaping robusto de argumentos Windows (regras CommandLineToArgvW) ou evitar composição manual de linha de comando.

### PERF-M1 — `path_belongs_to_inactive_panel`: varredura O(n) por resultado de thumbnail
**Arquivo:** [`helpers.rs#L168-L182`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/state/helpers.rs#L168-L182)

Faz scan linear de todos os itens visíveis do painel inativo. Chamada por cada thumbnail recebido (em `thumbnail_uploads.rs`/`thumbnail_workers.rs`), resultando em O(n·m) sob chegada rápida de thumbnails em layout dual-panel.

**Recomendação:** manter um `FxHashSet<PathBuf>` (ou `HashSet` de chaves normalizadas) dos paths visíveis do painel inativo, atualizado quando o snapshot muda; lookup O(1).

### PERF-M2 — `attempted_thumbnail_bucket`: `.clear()` bruto causa "thundering herd"
**Arquivo:** [`cache.rs#L457-L474`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/cache.rs#L440-L500)

Ao exceder `MAX_DYNAMIC_TEXTURE_CACHE_ITEMS * 2`, o mapa inteiro é limpo de uma vez, descartando todo o rastreamento e provocando re-extração de thumbnails para todos os itens visíveis simultaneamente.

**Recomendação:** usar uma estrutura LRU limitada (como já feito em `texture_cache`) em vez de `clear()` total.

### PERF-M3 — HashMaps/Sets sem limite explícito entre eventos de limpeza
**Arquivos:** [`state/mod.rs#L141`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/state/mod.rs#L136-L146) (`thumbnail_request_epochs`), e sets como `loading_icons` / `metadata_loading` referenciados em [`helpers.rs#L468-L472`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/state/helpers.rs#L460-L500)

Crescem a cada path único visitado; a limpeza via `retain` só ocorre em eventos específicos de gestão de memória. Entre esses eventos, navegar por muitas pastas acumula entradas sem teto.

**Recomendação:** limitar por LRU ou fazer poda por tamanho além da poda por evento; garantir remoção em falhas silenciosas (icons que nunca resolvem).

---

## 5. 🔵 Baixo / Informativo

### BUG-L1 — `unreachable!()` em `derive_output_path`
[`archive_extract.rs#L139`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/archive_extract.rs#L102-L141): `MatchKind::None => unreachable!()`. Seguro hoje, mas vira panic em runtime se o conjunto de variantes evoluir. Preferir retornar `Err`.

### BUG-L2 — `unwrap()` frágil no worker de thumbnails
[`worker.rs#L463 / L513`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/thumbnail/worker.rs#L420-L518): `active_bulk_session.unwrap()` protegido por `participates_in_bulk_scan`. Logicamente seguro, mas frágil; usar o valor já desempacotado numa variável local eliminaria o padrão.

### BUG-L3 — Parsing de USN com `try_into().unwrap()` sobre buffer do kernel
[`usn_journal.rs#L312, L344-L365`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/crates/mtt-search-service/src/usn_journal.rs#L295-L375): unwraps sobre fatias de tamanho fixo dentro de limites verificados. Baixo risco (o kernel retorna estrutura válida e há checagens de bounds), mas um journal corrompido é entrada não totalmente confiável; considerar `try_into().ok()` com `continue`.

### BUG-L4 — `let _ = send(...)` em invalidação de cache de disco
[`file_op_events.rs#L453-L497`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/message_handler/file_op_events.rs#L450-L497): falha de envio no canal de invalidação é ignorada. Aceitável se o receptor morreu, mas vale logar em nível debug.

### PERF-L1 — Clones por frame em `list_bridge`
[`list_bridge.rs#L268-L271`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/operations/ui_rendering/list_bridge.rs#L260-L300): `selected_file.clone()` e `renaming_state.clone()` por frame para contornar o borrow-checker. Impacto pequeno (structs modestas), mas evitável com reestruturação de borrows. (`items.clone()` é `Arc` — barato, sem problema.)

### PERF-L2 — `search_page`: varredura O(n) de todos os registros por consulta
[`file_index.rs#L920-L1000`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/crates/mtt-search-service/src/file_index.rs#L920-L1000): scan linear de todos os MFT records por query. Mitigado por deadline de 1,5s, SIMD (`memmem`) e fast-path ASCII, e roda no processo separado (não bloqueia a UI). Para índices muito grandes, considerar estrutura invertida/n-gram no futuro. Informativo.

### PERF-L3 — `normalize_search_path_key` aloca 2-3 Strings por chamada
[`results_panel.rs#L727-L739`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/global_search_overlay/results_panel.rs#L727-L740): chamado por resultado + por candidato. Ligado ao PERF-C1; ao mover o trabalho para fora do render, o impacto some.

### SEC-L1 — `trusted_file_manager_client`: verificação por basename + diretório irmão
[`ipc_authorization.rs#L110-L195`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/crates/mtt-search-service/src/ipc_authorization.rs#L110-L195): a confiança no cliente baseia-se no nome `mtt-file-manager.exe` residir no mesmo diretório do serviço. É um sinal fraco por si só (falsificável), **porém** a defesa real é o `ImpersonateNamedPipeClient` + checagem de acesso por path em cada consulta, o que torna isso defesa-em-profundidade aceitável (escrever na pasta do serviço em Program Files exige admin). **Informativo — sem ação urgente.**

### SEC-L2 — Parser IPC exposto a "Authenticated Users"
[`pipe_io.rs#L34-L143`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/crates/mtt-search-service/src/ipc_server/pipe_io.rs#L115-L187): qualquer usuário autenticado local pode conectar e exercitar o parser de requisições do serviço (que roda como LocalSystem). O parser é defensivo (limite de payload 64KB, validação, rate limit, watchdog anti-slowloris). Risco residual baixo, mas é a superfície de ataque mais sensível — recomenda-se fuzzing do `decode_message`/`SearchRequest::validate`. **Informativo.**

---

## 6. Pontos Fortes (o que está bem feito)

- **Isolamento do serviço de busca** em processo separado, com IPC via named pipe endurecido: ACL restrita a *Authenticated Users* + *LocalSystem*, `PIPE_REJECT_REMOTE_CLIENTS`, `FILE_FLAG_FIRST_PIPE_INSTANCE` (anti pipe-squatting), rate limiting global e por-PID, e watchdog anti-slowloris.
- **Autorização por consulta com impersonation** (`ImpersonateNamedPipeClient` + `CreateFileW`), impedindo divulgação de arquivos que o cliente não pode ler mesmo com o serviço rodando como SYSTEM. Cache de autorização por diretório é uma boa otimização.
- **Redação de paths** em mensagens de erro do serviço (`redact_paths`) evita vazamento de informação.
- **Hardening de DLL** (`SetDefaultDllDirectories`) contra DLL planting no serviço SYSTEM.
- **Supply-chain:** verificação de **SHA-256 fixado** do `pdfium.dll` no [`build.rs#L140-L173`](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/build.rs#L140-L173).
- **Validação de paths em camadas** (`sanitize_path`): checa null bytes, componentes `..`/`.`/`~`, ADS (`:`), nomes reservados do Windows, reparse points **antes** da canonicalização (ordem correta contra ataques de junction), e restrição por drive.
- **Zip Slip** tratado com sanitização de componentes + verificação `starts_with(dest_folder)` de defesa em profundidade.
- **Validação de path antes de spawnar** viewers de vídeo/PDF, com revalidação no processo filho (defesa em profundidade).
- **Resiliência:** uso extensivo de `catch_unwind` em workers e aritmética saturante em cálculos de índices/tamanhos.

---

## 7. Plano de Ação Priorizado

| # | Ação | Severidade | Esforço |
|---|---|---|---|
| 1 | Remover `std::fs::metadata` do render loop da busca (PERF-C1) — mover para worker ou `GetFileAttributesExW` | 🔴 Crítico | Médio |
| 2 | Corrigir injeção de comando no terminal admin (SEC-A1 + SEC-M1) | 🟠 Alto | Baixo |
| 3 | Tratar `join()`/pânico na inicialização (BUG-A1) | 🟠 Alto | Baixo |
| 4 | Logar/refletir falhas de escrita no banco (BUG-A2) | 🟠 Alto | Baixo |
| 5 | Cachear tags ordenadas (PERF-A1, PERF-A2) | 🟠 Alto | Baixo |
| 6 | Validar dimensões de thumbnail antes de alocar (BUG-M1) | 🟡 Médio | Baixo |
| 7 | Set de paths visíveis para painel inativo (PERF-M1) | 🟡 Médio | Baixo |
| 8 | LRU no `attempted_thumbnail_bucket` (PERF-M2) | 🟡 Médio | Baixo |
| 9 | Padronizar orderings atômicos em `window_subclass` (BUG-M3) | 🟡 Médio | Médio |
| 10 | Fuzzing do parser IPC (SEC-L2) | 🔵 Info | Médio |

---

*Fim do relatório.*
