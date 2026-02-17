# Async Consistency Probe para drives não-NTFS

## Context

Em drives não-NTFS (exFAT, FAT32), o `ReadDirectoryChangesW` do Windows **não notifica** mudanças feitas por outros processos. O fallback atual (`maybe_poll_non_usn_consistency`) faz um `read_directory_hdd_optimized` **síncrono na UI thread** a cada 45 segundos, causando:
- Detecção lenta de mudanças externas (~45s+)
- Bloqueio da UI thread durante disk read (50-300ms em USB lento)

**Objetivo**: Mover o probe para background thread + usar intervalos adaptativos menores.

---

## 1. Criar worker de consistency probe

**Novo arquivo**: `src/app/init_workers/consistency_probe_worker.rs`

Structs de comunicação:
```rust
pub struct ConsistencyProbeRequest {
    pub path: PathBuf,
    pub is_onedrive: bool,
    pub ui_signature: u64,
}

pub struct ConsistencyProbeResult {
    pub path: PathBuf,
    pub disk_signature: u64,
    pub path_vanished: bool,
}
```

Worker (Pattern B — long-lived loop, drena requests stale como `folder_size_worker`):
- Single thread, `recv()` blocks
- Drena requests pendentes, só processa o mais recente
- Chama `read_directory_hdd_optimized` + `compute_entries_signature`
- Compara `disk_signature != ui_signature` antes de enviar (evita msg desnecessária)
- `ctx.request_repaint()` após enviar resultado
- `IOPriority::Background` para não competir com UI

**Registrar módulo**: `src/app/init_workers/mod.rs`

---

## 2. Adicionar campos ao state

**Arquivo**: `src/app/state.rs` (junto dos campos `watcher_fallback_*`)

```rust
pub consistency_probe_tx: mpsc::Sender<ConsistencyProbeRequest>,
pub consistency_probe_rx: mpsc::Receiver<ConsistencyProbeResult>,
```

---

## 3. Spawnar worker no init

**Arquivos afetados**:
- `src/app/init_bootstrap.rs` — spawnar worker, passar handles
- `src/app/init_state_builders.rs` — receber e atribuir tx/rx
- `src/app/init.rs` — wiring

---

## 4. Refatorar `maybe_poll_non_usn_consistency`

**Arquivo**: `src/app/operations/message_handler/watcher_events.rs`

Dividir em duas funções:

### 4a. `maybe_send_consistency_probe()` — substitui o probe síncrono
- Mesmos guards (fallback_polling, loading, file_ops, minimized, etc.)
- Verifica intervalo adaptativo
- Computa `ui_signature` de `self.all_items` (in-memory, barato)
- Envia `ConsistencyProbeRequest` via `consistency_probe_tx`

### 4b. `process_consistency_probe_results()` — recebe resultados
- Chamada no início de `process_watcher_events_and_auto_reload`
- `try_recv()` no `consistency_probe_rx`
- Descarta se `result.path != current_path` (stale)
- Se `path_vanished`: `navigate_to_nearest_valid_ancestor()`
- Se signatures diferem: marca `rdcw_unreliable_drives`, invalida caches, `pending_auto_reload = true`

### 4c. `compute_entries_signature` → tornar acessível ao worker
- Mover para helper compartilhado ou duplicar a lógica no worker (função é ~20 linhas)

---

## 5. Intervalos adaptativos

**Arquivo**: `watcher_events.rs` — `fallback_poll_interval`

| Condição | Antes | Depois |
|---|---|---|
| non-USN, known_bad, ≤500 items | 30s | 10s |
| non-USN, known_bad, ≤2000 items | 30s | 15s |
| non-USN, known_bad, >2000 items | 45s | 25s |
| non-USN, unverified | 45s | 20s |

Seguro porque o disk read agora roda em background.

---

## Arquivos a modificar

1. **NOVO** `src/app/init_workers/consistency_probe_worker.rs`
2. `src/app/init_workers/mod.rs`
3. `src/app/init_bootstrap.rs`
4. `src/app/init_state_builders.rs`
5. `src/app/init.rs`
6. `src/app/state.rs`
7. `src/app/operations/message_handler/watcher_events.rs` (principal)

---

## Verificação

1. `cargo build` + `cargo test`
2. Teste manual: USB exFAT → deletar arquivo pelo Explorer → verificar atualização em ~10-20s sem stutter
3. Logs `[FS-WATCH-FALLBACK]` confirmam detecção de drift
4. Em NTFS: probe NÃO roda (`watcher_fallback_polling = false`)
