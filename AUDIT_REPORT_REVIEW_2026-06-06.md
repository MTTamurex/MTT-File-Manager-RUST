# Relatório de Auditoria Revisado - Validação do `AUDIT_REPORT.md`

Data da revisão: 2026-06-06

## 1. Sumário Executivo

O `AUDIT_REPORT.md` original continua útil, mas precisa ser atualizado. A maioria dos achados principais é válida, especialmente os riscos de bloqueio da UI e vazamento de `HBITMAP` em caminhos de erro. Porém, alguns itens estão desatualizados, superestimados ou com classificação incorreta.

Principais correções desta revisão:

- As métricas do projeto estão desatualizadas: hoje há `387` arquivos Rust rastreados e `104.543` linhas Rust, não cerca de `359` arquivos e `83.900` LOC.
- `ImageViewerApp` tem aproximadamente `212` campos públicos, então o problema de god struct é ainda maior do que o relatório original indicava.
- O worker principal de ícones já usa pool limitado; a crítica de thread por request só se aplica a fluxos auxiliares de ícones jumbo/drive/pastas especiais.
- `metadata::image::is_image_extension()` não é dead code; ele é chamado por `src/infrastructure/windows/metadata/mod.rs`.
- O achado sobre `GetLastError()` não foi confirmado como bug: nos trechos citados o erro é capturado imediatamente após `DeviceIoControl().is_err()`, antes de chamadas Win32 visíveis.
- `ext_key_stack()` não causa stack buffer overflow em Rust seguro; o problema real é panic em release para extensões longas.

## 2. Achados Críticos e de Alto Impacto

| ID original | Status | Veredito revisado |
|---|---|---|
| CRIT-01 | Válido | Há vazamento de `HBITMAP` em 9 sites que chamam `hbitmap_to_rgba(hbitmap)?` antes de `DeleteObject`. Se a conversão falhar, `?` pula o cleanup. |
| CRIT-02 | Válido | `std::fs::metadata()` roda na UI durante tooltip da busca global. Risco real de travamento em OneDrive/cloud-only. |
| CRIT-03 | Válido | O tooltip da busca global faz leitura SQLite, decode WebP e upload de textura dentro do callback de renderização. |
| CRIT-04 | Parcialmente válido | Há risco de panic em release para extensões longas, mas não é stack buffer overflow. Rust impede corrupção de memória nesse caso. |

### CRIT-01 - Vazamento de `HBITMAP` em erro

Confirmado nos 9 sites abaixo:

| Arquivo | Linhas atuais | Observação |
|---|---:|---|
| `src/workers/thumbnail/extraction/stage3_shell_api.rs` | 66-67 | `hbitmap_to_rgba(hbitmap)?` antes de `DeleteObject`. |
| `src/infrastructure/windows/icons/thumbnails.rs` | 27-30 | `GetImage` retorna `HBITMAP` owned. |
| `src/infrastructure/windows/icons/thumbnails.rs` | 64-66 | Mesmo padrão. |
| `src/infrastructure/windows/icons/thumbnails.rs` | 72-74 | Mesmo padrão. |
| `src/infrastructure/windows/icons/special.rs` | 86-90 | Mesmo padrão. |
| `src/infrastructure/windows/icons/special.rs` | 158-162 | Mesmo padrão. |
| `src/infrastructure/windows/icons/file_icons.rs` | 87-91 | Mesmo padrão. |
| `src/infrastructure/windows/icons/file_icons.rs` | 157-161 | Mesmo padrão. |
| `src/infrastructure/windows/icons/file_icons.rs` | 227-231 | Mesmo padrão. |

Não contar como vazamento os `HBITMAP` obtidos via `ISharedBitmap::GetSharedBitmap()` em `icons/thumbnails.rs`, porque a propriedade permanece com `ISharedBitmap`.

Correção recomendada:

```rust
let result = hbitmap_to_rgba(hbitmap);
let _ = DeleteObject(hbitmap.into());
let (rgba_data, width, height) = result?;
```

### CRIT-02 - Metadata bloqueante na UI

Confirmado em `src/ui/global_search_overlay/result_row.rs:331-362`.

O código executa `std::fs::metadata(&full_path)` no caminho de hover tooltip. O próprio projeto documenta o risco de `metadata()` em OneDrive em `src/ui/app/panels/content.rs:338-341`.

Correção recomendada: mover leitura de tamanho/data para worker assíncrono com cache e renderizar tooltip parcial enquanto o dado não chega.

### CRIT-03 - SQLite + WebP decode + textura na UI

Confirmado em `src/ui/global_search_overlay/result_row.rs:381-396`.

`app.disk_cache.get_latest(&p)` bloqueia em SQLite no thread da UI. Em seguida, `image::load_from_memory_with_format(... WebP)` decodifica a imagem e `ui.ctx().load_texture(...)` cria textura no mesmo callback.

Correção recomendada: buscar e decodificar em worker; a UI deve apenas consumir um resultado pronto e fazer upload com orçamento por frame.

### CRIT-04 - `ext_key_stack()`

Confirmado parcialmente em `src/ui/icon_loader/file_icons.rs:8-18`.

Problema real: `debug_assert!(len <= 32)` desaparece em release, e as fatias `buf[..ext_str.len()]` / `buf[ext_str.len()..len]` podem causar panic se a extensão + sufixo passar de 32 bytes.

Classificação revisada: bug de robustez / crash em release, não overflow de memória.

Correção recomendada: usar guarda runtime e fallback heap para chaves longas, ou aumentar/remover o buffer fixo.

## 3. Performance

| ID original | Status | Veredito revisado |
|---|---|---|
| PERF-01 | Válido com correção incompleta | `DirectoryCache::put()` reconstrói `BTreeSet` e recalcula `total_items()` em O(n). A solução precisa também inserir/remover incrementalmente e tratar evicções do LRU. |
| PERF-02 | Válido | `DirectoryIndex` usa `prepare()` em leituras repetidas (`get_directory` e `try_get_directory`). |
| PERF-03 | Parcialmente válido | `std::sync::Mutex` existe em caches SQLite, mas o impacto é baixo e a troca exige revisar semântica de poisoning/fallback. |
| PERF-04 | Parcialmente válido | O worker principal de ícones já é limitado; o problema fica restrito a fluxos auxiliares com `std::thread::spawn`. |
| PERF-05 | Válido | `stage3_shell_api.rs` possui `hbitmap_to_rgba` duplicado sem validação de dimensão. |
| PERF-06 | Válido | Há sobreposição real de lógica adaptativa entre `helpers.rs` e `thumbnail_uploads.rs`. |

### PERF-01 - `DirectoryCache::put()`

Confirmado em `src/infrastructure/directory_cache.rs:35-36`, `104` e `125`.

O relatório original está correto sobre o custo do rebuild. Porém, a correção proposta é incompleta: remover `sync_ordered_keys()` sem inserir a nova `path` em `ordered_keys` quebraria `invalidate_children()`. Também é preciso tratar a chave eventualmente evictada por `LruCache::put()`.

### PERF-02 - `DirectoryIndex::prepare()`

Confirmado em `src/infrastructure/directory_index.rs:87` e `135`.

Trocar por `prepare_cached()` é razoável para essas consultas repetitivas.

### PERF-03 - `std::sync::Mutex`

Confirmado em:

| Arquivo | Uso |
|---|---|
| `src/infrastructure/disk_cache.rs` | `Arc<Mutex<Connection>>` para reader/writer. |
| `src/infrastructure/directory_index.rs` | `Mutex<Connection>`. |
| `src/infrastructure/icon_disk_cache.rs` | `Mutex<Connection>` e `Mutex<()>`. |

Classificação revisada: dívida de consistência/performance baixa, não bug de produção. A troca para `parking_lot::Mutex` precisa ajustar retornos que hoje dependem de `lock().ok()?` e mensagens sobre poisoning.

### PERF-04 - Thread por request no loader de ícones

O achado original está superestimado.

O fluxo principal de ícones usa pool limitado em `src/app/init_workers/visual_workers.rs:126` e `153-155`, com `worker_count = cpu.clamp(2, 4)`. `request_icon_load()` envia para esse worker em `src/app/operations/thumbnails.rs:359-388`.

Ainda há spawns auxiliares sem limite em:

| Arquivo | Linhas | Fluxo |
|---|---:|---|
| `src/ui/icon_loader/async_ops.rs` | 73 | Ícone de drive. |
| `src/ui/icon_loader/async_ops.rs` | 111 | Ícone de pasta por path. |
| `src/ui/icon_loader.rs` | 235 | Ícone jumbo assíncrono do preview. |

Classificação revisada: risco médio-baixo e escopo menor que o relatório original.

### PERF-05 - `hbitmap_to_rgba` duplicado sem guardas

Confirmado em `src/workers/thumbnail/extraction/stage3_shell_api.rs:80-129`.

A versão compartilhada em `src/infrastructure/windows/bitmap_conversion.rs:25-27` valida dimensões zero e limites de `16384`. A duplicada não valida e pode alocar memória demais se `GetObjectW` retornar dimensões inválidas/corrompidas.

Correção recomendada: remover a duplicação e usar a função compartilhada.

### PERF-06 - Lógica adaptativa duplicada

Confirmado por sobreposição entre:

| Arquivo | Evidência |
|---|---|
| `src/app/state/helpers.rs` | `current_dynamic_texture_keep_count`, `current_thumbnail_rgba_budget_bytes`, limites de pending thumbnails e funções `dynamic_*`. |
| `src/app/operations/message_handler/thumbnail_uploads.rs` | `compute_texture_cache_target_items`, `live_frame_pressure_ms`, retuning de cache e pressão de frame. |

Classificação revisada: dívida de manutenção, não problema urgente.

## 4. Arquitetura

| ID original | Status | Veredito revisado |
|---|---|---|
| ARCH-01 | Válido | `ImageViewerApp` tem cerca de `212` campos públicos, não apenas `170+`. |
| ARCH-02 | Válido, incompleto | O domínio depende de infraestrutura em mais pontos do que o relatório listou. |
| ARCH-03 | Válido | A camada `app` importa tipos de `ui` extensivamente. |
| ARCH-04 | Válido, métricas desatualizadas | Hoje são `46` arquivos Rust acima de 500 linhas; `4` estão acima de 1000 linhas. |

### ARCH-01 - God struct

Confirmado em `src/app/state/mod.rs:73-509`.

Contagem atual aproximada: `212` campos públicos em `ImageViewerApp`.

Classificação revisada: válido e subestimado pelo relatório original.

### ARCH-02 - Domínio dependendo de infraestrutura

Confirmado e incompleto no relatório original.

| Arquivo | Dependência |
|---|---|
| `src/domain/file_entry.rs:1` | `DriveType` vindo de `infrastructure::windows::system_info`. |
| `src/domain/file_entry.rs:105` | `is_media_extension` vindo de `infrastructure::windows`. |
| `src/domain/thumbnail.rs:1` | `IOPriority` vindo de `infrastructure::io_priority`. |
| `src/domain/errors.rs:10` | `SecurityError` vindo de `infrastructure::security`. |

### ARCH-03 - App importando UI

Confirmado em `src/app/state/mod.rs` e outros módulos de `src/app`.

Exemplos diretos em `state/mod.rs`:

| Linha | Tipo de UI |
|---:|---|
| 21 | `FxHashSet` reexportado por `ui::cache`. |
| 45 | `MediaPreview`. |
| 46 | `ContextMenuState`. |
| 47 | `IconLoader`. |
| 48 | `SvgIconManager`. |
| 132 | `CacheManager`. |
| 169-170 | `RectangleSelectionState`. |
| 187 | `GifPlayer`. |
| 322 | `GifManager`. |
| 412-413 | `PendingOperations` e `ScrollPredictor`. |

### ARCH-04 - Arquivos grandes

Métrica atualizada: `46` arquivos Rust em `src/` acima de 500 linhas; `4` acima de 1000 linhas.

Maiores arquivos atuais:

| Linhas | Arquivo |
|---:|---|
| 1443 | `src/app/operations/message_handler/thumbnail_uploads.rs` |
| 1328 | `src/app/state/helpers.rs` |
| 1167 | `src/ui/cache.rs` |
| 1047 | `src/infrastructure/archive_extract.rs` |
| 986 | `src/workers/thumbnail/queue.rs` |
| 918 | `src/ui/app/panels/content.rs` |
| 906 | `src/app/operations/message_handler/helpers.rs` |
| 901 | `src/ui/sidebar.rs` |
| 873 | `src/image_viewer/app/mod.rs` |
| 858 | `src/ui/toolbar.rs` |
| 828 | `src/video_player/mod.rs` |
| 799 | `src/infrastructure/diagnostic_logger.rs` |

## 5. Tratamento de Erros

| ID original | Status | Veredito revisado |
|---|---|---|
| ERR-01 | Válido | `start_folder_load_pipeline()` descarta falha de spawn. |
| ERR-02 | Válido | `GifManager::new()` descarta falha de spawn dos workers. |
| ERR-03 | Válido | `spawn_folder_preview_worker()` descarta falha de spawn. |
| ERR-04 | Não confirmado | Não há chamada Win32 visível entre `DeviceIoControl().is_err()` e `GetLastError()`. |

### ERR-01 a ERR-03 - Falha de spawn descartada

Confirmado em:

| Arquivo | Linhas | Observação |
|---|---:|---|
| `src/app/operations/folder_loading/load_pipeline.rs` | 38-40 | `let _ = Builder::new().spawn(...)`. |
| `src/ui/components/gif_manager.rs` | 105-124 | `let _ = Builder::new().spawn(...)`. |
| `src/workers/folder_preview_worker.rs` | 184-187 | `let _ = Builder::new().spawn(...)`. |

Correção recomendada: checar o `Result`, logar erro e, quando aplicável, enviar resposta de falha para desbloquear UI.

### ERR-04 - `GetLastError()`

Reclassificado como não confirmado.

Os locais citados existem:

| Arquivo | Linhas |
|---|---:|
| `crates/mtt-search-service/src/mft_reader.rs` | 207-218 e 471-482 |
| `crates/mtt-search-service/src/usn_journal.rs` | 189-200 e 282-305 |

Mas o padrão atual captura `GetLastError()` imediatamente após `result.is_err()`. Não foi encontrada chamada Win32 intermediária antes da captura. É aceitável refatorar para `match result { Err(error) => ... }` e extrair o código do erro do `windows-rs`, mas o relatório original exagera ao chamar isso de diagnóstico incorreto comprovado.

## 6. Concorrência

| ID original | Status | Veredito revisado |
|---|---|---|
| CONC-01 | Válido, baixo impacto | Timeout de join de 300 ms é apertado para loop com sleep de 250 ms. |
| CONC-02 | Parcialmente válido | Existe `catch_unwind(AssertUnwindSafe)`, mas um dos arquivos citados não existe mais. |
| CONC-03 | Válido, baixo impacto | `IconLoader` usa `std::sync::mpsc::channel()` sem limite. |

### CONC-01 - Timeout de join do MPV

Confirmado em `src/ui/components/mpv/event_loop.rs:218-224`.

O loop dorme 250 ms (`event_loop.rs:202-203`) e o join espera até 300 ms. Aumentar para 500 ms é uma correção simples e razoável.

### CONC-02 - `catch_unwind(AssertUnwindSafe)`

Parcialmente válido.

Encontrado em:

| Arquivo | Linhas |
|---|---:|
| `src/ui/components/mpv_preview/playback_state.rs` | 100-102 |
| `src/ui/components/media_preview.rs` | 143-146 |

O arquivo `src/ui/components/mpv_preview/controls.rs` citado no relatório original não existe na árvore atual. O achado permanece como cleanup de baixo impacto: se mantiver `catch_unwind`, deve logar; se remover, o fluxo fica mais simples.

### CONC-03 - Canal sem limite em `IconLoader`

Confirmado em `src/ui/icon_loader.rs:128`.

Risco baixo porque há deduplicação por chave e limite de uploads por frame em `poll_async_icons()`, mas um canal limitado continuaria sendo mais defensivo.

## 7. Dead Code

| Item original | Status revisado | Evidência |
|---|---|---|
| `application::filter_items()` | Válido | Sem chamadas exatas encontradas fora da definição em `src/application/mod.rs:24-29`. |
| `application::filter_items_opt()` | Válido | Sem chamadas exatas encontradas fora da definição em `src/application/mod.rs:33-38`. |
| `application::filter_items_cow()` | Válido | Sem chamadas exatas encontradas fora da definição em `src/application/mod.rs:41-46`. |
| `ui::views::common::format_date()` | Válido | Sem chamadas exatas encontradas fora da definição em `src/ui/views/common.rs:45-47`. |
| `ui::views::common::format_size()` | Válido | Sem chamadas exatas encontradas fora da definição em `src/ui/views/common.rs:50-52`. |
| `application::file_operations::delete_with_shell()` | Válido | Função deprecated em `src/application/file_operations.rs:109-119`, sem chamada exata encontrada. |
| `application::file_operations::rename_with_shell()` | Válido | Função deprecated em `src/application/file_operations.rs:133-152`, sem chamada exata encontrada. |
| `metadata::image::is_image_extension()` | Inválido como dead code | Chamado por `src/infrastructure/windows/metadata/mod.rs:101`. É redundante, mas não dead code. |

## 8. Quick Wins Revisados

| Prioridade | Ação | Motivo |
|---|---|---|
| Alta | Corrigir cleanup de `HBITMAP` nos 9 sites owned. | Evita vazamento de GDI handle em erro. |
| Alta | Remover `metadata()` síncrono do tooltip da busca global. | Evita travamento de UI em OneDrive/cloud-only. |
| Alta | Mover leitura SQLite/decode WebP do tooltip para worker. | Evita stutter de frame e I/O no render. |
| Alta | Adicionar guarda runtime/fallback em `ext_key_stack()`. | Evita panic em release para extensões longas. |
| Média | Remover `hbitmap_to_rgba` duplicado em `stage3_shell_api.rs`. | Reusa validação de dimensão já existente. |
| Média | Corrigir `DirectoryCache::put()` com updates incrementais completos. | Reduz mutex hold time sem quebrar `ordered_keys`. |
| Média | Tratar falhas de `thread::Builder::spawn()`. | Melhora diagnóstico e evita estado silenciosamente travado. |
| Baixa | Trocar `prepare()` por `prepare_cached()` em `DirectoryIndex`. | Reduz overhead em chamadas repetidas. |
| Baixa | Remover wrappers/funções deprecated realmente sem uso. | Reduz ruído de manutenção. |
| Baixa | Avaliar `parking_lot::Mutex` nos caches SQLite. | Consistência e pequena melhoria de lock, com cuidado na semântica. |

## 9. Conclusão

O relatório original deve ser substituído conceitualmente por esta versão revisada. Os problemas mais importantes são reais, mas a priorização precisa mudar:

- Manter como prioridade máxima: vazamento de `HBITMAP`, bloqueios na UI da busca global e panic de `ext_key_stack()`.
- Rebaixar: thread-per-request do icon loader, `std::sync::Mutex`, canal sem limite e `catch_unwind`.
- Remover ou reescrever: `GetLastError()` como bug comprovado e `metadata::image::is_image_extension()` como dead code.
- Atualizar métricas: tamanho do projeto, arquivos grandes e contagem de campos de `ImageViewerApp`.
