# Auditoria Completa — MTT File Manager (Rust)

**Data:** 2026-04-18
**Escopo:** Código fonte completo, unsafe/FFI, Windows API, concorrência, I/O, performance, arquitetura

---

## 1. Resumo Geral

**Estado do projeto:** O codebase é maduro e demonstra conhecimento sólido de Rust, Windows API e arquitetura de sistemas. A segurança é tratada com defesa em profundidade (IPC, archive extraction, path validation). O ciclo de vida (shutdown, startup faseado) é bem engenhado. O uso de `unsafe` é concentrado em integrações legítimas com Win32/COM/FFI.

**Principais riscos técnicos:**
1. **Undefined behavior** por desalinhamento de ponteiros em 3 locais (ntfs_reader, buffer_parser)
2. **Alocações excessivas** no hot path de carregamento de diretórios (~3-4x cópias de FileEntry)
3. **Struct `ImageViewerApp`** monolítica com 90+ campos — risco de manutenção e regressão
4. **Lock poisoning** desabilita silenciosamente caches sem logging
5. **Use-after-free** no drive watcher (mitigado — feature desabilitada por padrão, opt-in via env var)

---

## 2. Problemas Críticos

### ~~CRIT-01~~ → ~~BAIXO-01: Use-after-free — I/O overlapped não drenada antes de drop do buffer~~ ✅ CORRIGIDO
- **Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs`
- **Impacto:** ~~Crítico~~ → **Baixo** — a integração app-level do DriveWatcher foi **removida completamente**. O módulo `drive_watcher.rs` (+ submodules) permanece apenas como dependência interna do `user_session_search` para monitorar volumes FUSE/virtuais. O código afetado não é atingido pelo fluxo principal da aplicação.
- **Correção aplicada:** `GetOverlappedResult(handle, &overlapped, &mut dummy, true)` no shutdown para drenar I/O pendente antes de liberar o buffer.

### CRIT-02: ComGuard ignora falha de CoInitializeEx
- **Arquivo:** `src/workers/folder_preview_worker.rs`
- **Impacto:** Crítico — UB por chamar `CoUninitialize` sem `CoInitializeEx` bem-sucedido
- **Causa:** `ComGuard` sempre chama `CoUninitialize` no `Drop`, mesmo quando `CoInitializeEx` retornou erro. Violar o contrato COM causa UB.
- **Correção:** Tracker booleano como em `file_operation_worker.rs`:
```rust
struct ComGuard { initialized: bool }
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized { unsafe { CoUninitialize(); } }
    }
}
```

### CRIT-03: Icon worker COM sem RAII guard
- **Arquivo:** `src/app/init_workers/visual_workers.rs` (~L278)
- **Impacto:** Crítico — `CoUninitialize` nunca chamado se thread panic fora do `catch_unwind` per-item
- **Causa:** COM init/uninit manual sem guard RAII, diferente dos outros workers que usam `ComGuard`.

---

## 3. Problemas de Segurança (Rust / Unsafe / FFI)

### SEC-01: Alignment UB — `FileDirectoryInfo` de buffer não-alinhado
- **Arquivo:** `src/infrastructure/ntfs_reader.rs` (~L136)
- **Impacto:** Alto — UB formal, benigno em x86 mas pode ser explorado pelo compilador
- **Causa:** `Vec<u8>` (alinhamento=1) castado para `*const FileDirectoryInfo` (requer alinhamento=8).
- **Correção:** Buffer com alinhamento garantido:
```rust
#[repr(C, align(8))]
struct AlignedBuffer([u8; BUFFER_SIZE]);
```

### SEC-02: Alignment UB — `FILE_NOTIFY_INFORMATION` de buffer não-alinhado
- **Arquivo:** `src/infrastructure/drive_watcher/buffer_parser.rs` (~L30)
- **Impacto:** Alto — mesma classe de UB que SEC-01
- **Correção:** Mesma abordagem — buffer alinhado ou `read_unaligned`.

### SEC-03: `hbitmap_to_rgba` sem limite de dimensões
- **Arquivo:** `src/infrastructure/windows/bitmap_conversion.rs`
- **Impacto:** Médio — OOM com bitmap adversarial; overflow de `width * height * 4` em 32-bit
- **Correção:**
```rust
if width > 16384 || height > 16384 || width == 0 || height == 0 {
    return Err("Invalid bitmap dimensions".into());
}
```

### SEC-04: `NameArena::get` panic com `NameRef` inválido
- **Arquivo:** `crates/mtt-search-service/src/name_arena.rs` (~L81)
- **Impacto:** Médio — crash do search service (processo SYSTEM)
- **Causa:** Indexação de slice sem bounds check. `get_lowered` já tem o guard, `get` não.
- **Correção:** `if end > self.buf.len() { return ""; }`

### ~~SEC-05: NV12→RGBA panic com dimensões ímpares~~ ✅ CORRIGIDO
- **Arquivo:** `src/workers/thumbnail/processing/format_conversion.rs` (~L15)
- **Impacto:** Médio — panic na thread de thumbnail worker
- **Causa:** Cálculo de stride UV assume dimensões pares; dimensões ímpares produzem index out of bounds.
- **Correção aplicada:** Clamp UV coords para dimensões pares com `& !1` + `.min()` safety.

### SEC-06: `GlobalAlloc` memory leak no clipboard
- **Arquivo:** `src/infrastructure/windows_clipboard.rs` (~L137-L148)
- **Impacto:** Médio — leak de `HGLOBAL` se `GlobalLock` ou `SetClipboardData` falha
- **Causa:** Sem `GlobalFree` nos error paths.

### ~~SEC-07: Pipe squatting na IPC do image viewer~~ ✅ CORRIGIDO
- **Arquivo:** `src/image_viewer/ipc.rs` (~L85-L100)
- **Impacto:** Médio — processo malicioso local pode interceptar caminhos de arquivo
- **Causa:** Pipe destruído e recriado entre clientes; janela de race para squatting.
- **Correção aplicada:** Pipe criado uma vez fora do loop e reutilizado via `DisconnectNamedPipe` + `ConnectNamedPipe`. Elimina race window entre `CloseHandle` e `CreateNamedPipeW`.

---

## 4. Problemas de Performance

### PERF-01: `filter_items()` clona `all_items` em toda chamada
- **Arquivo:** `src/app/operations/folder_loading/view_updates.rs` (~L20)
- **Impacto:** Alto — ~10MB alocações por folder load com 50K arquivos
- **Causa:** `self.items = Arc::new(self.all_items.clone())` mesmo sem filtro ativo.
- **Correção:** Quando query vazia, compartilhar via `Arc` sem clonar.

### PERF-02: Clone excessivo de `FileEntry` no pipeline de loading
- **Arquivos:** `src/app/operations/folder_loading/load_pipeline/tier3_fallback.rs`, `src/app/operations/folder_loading/load_pipeline/optimized_tiers.rs`, `src/infrastructure/directory_cache.rs`
- **Impacto:** Alto — 3-4 cópias completas do vetor de entries por folder load
- **Causa:** `entry.clone()` para accumulator + batch, `batch.clone()` para send, `.clone()` para cache.
- **Correção:** `std::mem::take(batch)` consistente; `Arc<Vec<FileEntry>>` para transfers cache→sender.

### PERF-03: `FileEntry` struct inflada (~240 bytes + heap)
- **Arquivo:** `src/domain/file_entry.rs`
- **Impacto:** Alto — `name` duplica `path`; `drive_info`/`deletion_date`/`recycle_original_path` pagam custo em todo entry mesmo quando não usados
- **Correção:** `Box<RecycleBinMeta>` para campos do recycle bin; accessor computado para `name`.

### PERF-04: Verificação de mtime de subpastas sem limite no fast path
- **Arquivo:** `src/app/operations/folder_loading/load_pipeline/fast_paths.rs` (~L60)
- **Impacto:** Médio-Alto — 500 subpastas = 500 syscalls no path que deveria ser instantâneo
- **Correção:** Limitar a amostragem (primeiras 10-20 subpastas) ou pular em HDD.

### PERF-05: HDD reader "batched" lê tudo antes de dividir
- **Arquivo:** `src/infrastructure/windows/hdd_directory_reader.rs`
- **Impacto:** Médio — bloqueia UI thread até enumeration completa de 100K+ arquivos
- **Correção:** Streaming verdadeiro com callback/channel durante o loop FindFirstFile.

### PERF-06: `DirectoryIndex` — single Mutex bloqueia leituras durante write
- **Arquivo:** `src/infrastructure/directory_index.rs`
- **Impacto:** Médio — `DELETE FROM` + 10K `INSERT` segura Mutex durante toda transação
- **Correção:** WAL mode SQLite + connection pool; ou `RwLock` com writer separado.

### ~~PERF-07: Sort por Type aloca `OsString` por comparação~~ ✅ CORRIGIDO
- **Arquivo:** `src/application/sorting/sort_impl.rs` (~L110)
- **Impacto:** Médio-Baixo — ~260K alocações em sort de 10K itens
- **Correção aplicada:** Comparação zero-alloc byte-a-byte com `.to_ascii_lowercase()` em iterador sobre bytes da extensão extraída de `name` via `rsplit_once('.')`. Elimina ~260K alocações de `OsString`.

### ~~PERF-08: `path_matches_prefix` aloca 4-6 strings por evento~~ ✅ CORRIGIDO
- **Arquivo:** `src/infrastructure/drive_watcher/buffer_parser.rs` (~L78)
- **Impacto:** Médio — milhares de eventos/segundo em burst (OneDrive sync, copy)
- **Correção aplicada:** Reescrita completa usando `to_string_lossy()` (Cow, sem alloc para UTF-16 válido) + `eq_ignore_ascii_case` em byte slices. Zero alocações heap. Também corrige WIN-04 (case folding ASCII ordinal em vez de Unicode `to_lowercase()`).

### ~~PERF-09: `adaptive_batch` cálculo de `avg_time_per_item` incorreto~~ ✅ CORRIGIDO
- **Arquivo:** `src/infrastructure/adaptive_batch.rs` (~L60)
- **Impacto:** Baixo-Médio — batch sizing oscila incorretamente
- **Causa:** Denominador usa `items_processed` do batch atual × `batch_count` total, deveria ser total cumulativo. `Vec::remove(0)` é O(n), deveria ser `VecDeque`.
- **Correção aplicada:** `BatchSample` struct com (duration, items); cálculo correto `total_time/total_items`; `VecDeque` em vez de `Vec::remove(0)`.

---

## 5. Problemas na Integração com Windows API

### WIN-01: `GetDiskFreeSpaceW` overflow em volumes >16TB
- **Arquivo:** `src/infrastructure/windows/drives.rs` (~L333)
- **Impacto:** Alto — valores incorretos de espaço em disco em volumes NTFS grandes
- **Causa:** `GetDiskFreeSpaceW` retorna contadores de cluster 32-bit; wrap em >16TB com clusters 4K.
- **Correção:** Substituir por `GetDiskFreeSpaceExW` (retorna bytes 64-bit).

### ~~WIN-02: `WaitForSingleObject(process, INFINITE)` no elevated helper~~ ✅ CORRIGIDO
- **Arquivo:** `src/infrastructure/windows/drives.rs` (~L231)
- **Impacto:** Médio — thread bloqueada infinitamente se processo elevado trava/é morto pelo AV
- **Correção aplicada:** Timeout finito de 30s com mensagem específica para timeout vs erro genérico.

### WIN-03: `to_string_lossy()` corrompe paths não-UTF-8
- **Arquivos:** `src/infrastructure/ntfs_reader.rs` (~L90), `src/infrastructure/drive_watcher/buffer_parser.rs` (~L18)
- **Impacto:** Médio — paths com surrogates não-pareados (raro mas possível no Windows) são corrompidos
- **Correção:** `OsStr::encode_wide()` direto, sem round-trip por `&str`.

### ~~WIN-04: Unicode case folding diverge da semântica Windows~~ ✅ CORRIGIDO (via PERF-08)
- **Arquivo:** `src/infrastructure/drive_watcher/buffer_parser.rs` (~L80)
- **Impacto:** Médio — `str::to_lowercase()` usa Unicode folding; Windows usa ordinal
- **Correção aplicada:** Reescrita de `path_matches_prefix` em PERF-08 usa `eq_ignore_ascii_case` em byte slices — comparação ASCII ordinal, alinhada com semântica Windows.

### ~~WIN-05: `cancel_all_pending_io` usa `THREAD_TERMINATE` para `CancelSynchronousIo`~~ ✅ FALSO POSITIVO
- **Arquivo:** `src/ui/app/lifecycle.rs` (~L322)
- **Impacto:** Nenhum — `THREAD_TERMINATE` é o access right documentado pela Microsoft para `CancelSynchronousIo`.
- **Verificação:** [MSDN CancelSynchronousIo](https://learn.microsoft.com/en-us/windows/win32/fileio/cancelsynchronousio-func): "hThread — A handle to the thread. This handle must have the THREAD_TERMINATE access right."

### WIN-06: `RegisterDeviceNotificationW` handle nunca desregistrado
- **Arquivo:** `src/infrastructure/windows/device_change.rs` (~L98)
- **Impacto:** Baixo — cleanup automático no exit do processo; handle fica ativo desnecessariamente.

---

## 6. Problemas de Concorrência

### ~~CONC-01: `ThumbnailDiskCache` reader = writer.clone() — armadilha de deadlock~~ ✅ CORRIGIDO
- **Arquivo:** `src/infrastructure/disk_cache.rs` (~L136)
- **Impacto:** Crítico (latente) — se reader fallback para `writer.clone()`, qualquer path que segure writer e chame reader causa deadlock
- **Causa:** `Mutex` do Rust não é reentrante. Nenhum path atual causa o deadlock, mas uma edição descuidada pode.
- **Correção aplicada:** Comentário SAFETY INVARIANT + `log::warn` no fallback path documentando o risco de deadlock.

### CONC-02: Lock poisoning silencia falha permanente dos caches
- **Arquivos:** `src/infrastructure/directory_cache.rs`, `src/infrastructure/directory_dirty_registry.rs`
- **Impacto:** Alto — cache permanentemente inoperante sem log de erro
- **Causa:** `.lock().ok()?` retorna `None` silenciosamente em lock poisoned.
- **Correção:** `.unwrap_or_else(|e| e.into_inner())` (como thumbnail system usa) ou log de warning.

### ~~CONC-03: Threads detached sem limite no global search~~ ✅ CORRIGIDO
- **Arquivo:** `src/workers/global_search_worker.rs` (~L197)
- **Impacto:** Alto — digitação rápida + erros IPC = dezenas de threads bloqueadas acumulando
- **Correção aplicada:** `Arc<AtomicBool>` in-flight guard limita a 1 thread de total count por vez. `swap(true, AcqRel)` + RAII `InFlightGuard` com `Drop` que reseta o flag. Tasks extras são descartadas (generation check já invalida resultados stale).

### ~~CONC-04: `.expect()` em thread spawn = crash da aplicação~~ ✅ CORRIGIDO
- **Arquivos:** `src/app/init_workers/filesystem_workers.rs`, `src/app/init_workers/consistency_probe_worker.rs`
- **Impacto:** Alto — resource exhaustion faz spawn falhar → panic → crash
- **Correção aplicada:** `if let Err(e) = .spawn()` com log::error + degradação graceful (worker desabilitado, sends falham silenciosamente).

### CONC-05: Shared extension icon cache — 16 workers contendem em Mutex
- **Arquivo:** `src/app/init_workers/visual_workers.rs` (~L152)
- **Impacto:** Médio — cada hit clona `Vec<u8>` (4-16KB) segurando o lock
- **Correção:** `DashMap` ou caches per-worker com sync periódico.

### ~~CONC-06: GC worker demora até 180s para notar shutdown~~ ✅ FALSO POSITIVO
- **Arquivo:** `src/app/init_workers/background_jobs.rs`
- **Impacto:** Nenhum — `sleep_until_next_cycle` já faz polling de `GC_WORKER_RUNNING` a cada 1 segundo. O valor 180s (`GC_ACTIVE_INTERVAL_SECS`) é o intervalo total entre ciclos GC, dividido em polls de 1s. Latência máxima de shutdown = 1 segundo, não 180.

---

## 7. Problemas de Arquitetura

### ARCH-01: `ImageViewerApp` — god struct com 90+ campos
- **Arquivo:** `src/app/state/mod.rs`
- **Impacto:** Alto — impossível passar subset de estado; todo método recebe `&mut self` com acesso total
- **Correção:** Extrair sub-structs por domínio (`WatcherState`, `MediaState`, `DragDropState`) — padrão já validado por `DriveState`, `FolderSizeState`, etc.

### ARCH-02: 15 arquivos acima de 400 linhas (violação de AGENTS.md)

| Arquivo | Linhas | Prioridade |
|---------|--------|------------|
| ~~`src/app/operations/message_handler/watcher_drive_processing.rs`~~ | ~~953~~ | ~~Removido~~ |
| `src/app/shortcuts.rs` | **664** | Média |
| `src/infrastructure/archive_extract.rs` | **649** | Alta |
| `src/app/operations/message_handler/thumbnail_uploads.rs` | **648** | Média |
| `src/app/init.rs` | **572** | Média |
| `src/app/init_workers/filesystem_workers.rs` | **562** | Média |
| `src/ui/app/input.rs` | **546** | Baixa |
| `src/app/operations/message_handler/thumbnail_workers.rs` | **536** | Média |
| `src/app/operations/ui_rendering/list_bridge.rs` | **525** | Baixa |
| `src/app/operations/ui_rendering/grid_bridge.rs` | **477** | Baixa |
| `src/app/init_bootstrap.rs` | **466** | Média |
| `src/app/operations/context_menu.rs` | **438** | Baixa |
| `src/app/operations/message_handler/file_op_events.rs` | **428** | Baixa |
| `src/app/init_workers/fast_paths.rs` | **419** | Baixa |
| `src/app/operations/message_handler/watcher_events.rs` | **418** | Baixa |

### ARCH-03: Lógica de batch/cache duplicada em 3 tiers
- **Arquivos:** `src/app/operations/folder_loading/load_pipeline/fast_paths.rs`, `optimized_tiers.rs`, `tier3_fallback.rs`
- **Impacto:** Médio — qualquer mudança no protocolo de batch requer edição em 3+ locais idênticos

### ARCH-04: `UIState` vestigial duplica campos do `ImageViewerApp`
- **Arquivo:** `src/app/ui_state.rs`
- **Impacto:** Médio — campos como `selected_items`, `hovered_item`, `rename_text` existem nos dois; risco de divergência silenciosa

### ARCH-05: Funções com 19-21 argumentos no pipeline de loading
- **Arquivo:** `src/app/operations/folder_loading/load_pipeline.rs`
- **Impacto:** Baixo (manutenção) — difícil de refatorar, fácil de errar em alterações

---

## 8. Melhorias Recomendadas

| # | Melhoria | Impacto | Esforço |
|---|----------|---------|---------|
| 1 | Extrair sub-structs de `ImageViewerApp` (WatcherState, MediaState, DragDropState) | Alto | Médio |
| ~~2~~ | ~~RAII consistente para COM em todos workers (`ComGuard` com tracker booleano)~~ | ~~Alto~~ | ✅ CORRIGIDO |
| 3 | `Arc<Vec<FileEntry>>` para transferências cache→pipeline→UI sem clone | Alto | Médio |
| ~~4~~ | ~~`GetOverlappedResult` no shutdown do drive watcher~~ | ~~Alto~~ | ✅ CORRIGIDO |
| ~~5~~ | ~~Buffers alinhados para parsers de `NtQueryDirectoryFile` e `ReadDirectoryChangesW`~~ | ~~Alto~~ | ✅ CORRIGIDO |
| ~~6~~ | ~~Substituir `.lock().ok()?` por recover-from-poison com logging~~ | ~~Médio~~ | ✅ CORRIGIDO |
| ~~7~~ | ~~Bounds check em `NameArena::get`~~ | ~~Médio~~ | ✅ CORRIGIDO |
| ~~8~~ | ~~Dimension cap em `hbitmap_to_rgba`~~ | ~~Médio~~ | ✅ CORRIGIDO |
| ~~9~~ | ~~`GetDiskFreeSpaceExW` em vez de `GetDiskFreeSpaceW`~~ | ~~Médio~~ | ✅ CORRIGIDO |
| 10 | Streaming real no HDD directory reader | Médio | Médio |
| ~~11~~ | ~~Timeout finito no `WaitForSingleObject` do elevated helper~~ | ~~Médio~~ | ✅ CORRIGIDO |
| 12 | Remover/integrar `UIState` vestigial | Médio | Baixo |

---

## 9. Quick Wins (alto impacto / baixo esforço)

| # | Fix | Linhas de código | Impacto |
|---|-----|-----------------|---------|
| ~~1~~ | ~~`GetOverlappedResult(handle, &overlapped, &mut dummy, true)` no shutdown do drive watcher (opt-in, desabilitado por padrão)~~ | ✅ CORRIGIDO | ~~Elimina use-after-free (baixa prioridade — código inativo)~~ |
| ~~2~~ | ~~`#[repr(C, align(8))]` no buffer do ntfs_reader e buffer_parser~~ | ✅ CORRIGIDO | ~~Elimina UB de alinhamento~~ |
| ~~3~~ | ~~`ComGuard { initialized: bool }` no folder_preview e icon workers~~ | ✅ CORRIGIDO | ~~Elimina UB de COM~~ |
| ~~4~~ | ~~`if end > self.buf.len() { return ""; }` em `NameArena::get`~~ | ✅ CORRIGIDO | ~~Previne crash do search service~~ |
| ~~5~~ | ~~`if width > 16384 \|\| height > 16384` em `hbitmap_to_rgba`~~ | ✅ CORRIGIDO | ~~Previne OOM/overflow~~ |
| ~~6~~ | ~~`GlobalFree(hmem)` nos error paths do clipboard~~ | ✅ CORRIGIDO | ~~Elimina memory leak~~ |
| ~~7~~ | ~~Substituir `GetDiskFreeSpaceW` → `GetDiskFreeSpaceExW`~~ | ✅ CORRIGIDO | ~~Corrige volumes >16TB~~ |
| ~~8~~ | ~~`.unwrap_or_else(\|e\| e.into_inner())` em directory_cache~~ | ✅ CORRIGIDO | ~~Recupera de lock poison~~ |
| 9 | `filter_items()` share Arc sem clone quando query vazia | ~5 linhas | Elimina ~10MB allocs/folder |
| ~~10~~ | ~~Limitar subfolder mtime check a 20 entries no fast path~~ | ✅ CORRIGIDO | ~~Fix regression HDD~~ |
