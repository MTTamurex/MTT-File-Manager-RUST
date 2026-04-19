# Plano de Implementação — Correções da Auditoria

> Documento acompanha [AUDIT_REPORT.md](AUDIT_REPORT.md).
> Cada item é autocontido: **arquivo**, **o que fazer**, **como fazer**, **como validar**, **risco de regressão**.
> IDs seguem os do relatório de auditoria (C = Crítico, S = Segurança, P = Performance, W = Windows API, Co = Concorrência, A = Arquitetura).

> Status usados neste documento:
> - `IMPLEMENTADO`: concluído no código.
> - `JA IMPLEMENTADO`: já estava resolvido no código quando o plano foi revisado.
> - `PENDENTE`: ainda falta implementar.
> - `RECLASSIFICADO`: item deixou de ser pendência prática ou não afeta o runtime principal.

> Ultima validacao apos os itens marcados como implementados abaixo: `cargo check --workspace` OK.

---

## Status Atual

- **Implementado neste ciclo:** `F0.1`, `F0.2`, `F0.3`, `F1.1`, `F1.2`, `F1.3`, `F1.4`, `F1.6`, `F2.2`, `F2.3`, `F2.4`, `F2.5`, `F2.6`, `F3.1`, `F3.2`, `F3.3`, `F4.1`, `F4.2`, `F4.3`, `F4.4`, `F4.5`, `F4.6`, `F4.7`, `F5.1`, `F5.3`, `F5.4`, `F5.6`, `F5.7`, `F5.8`, `F5.9`.
- **Reclassificado:** `F1.5` (a ocorrencia auditada esta em codigo de teste, nao no fluxo de runtime da aplicacao).
- **Ja estava implementado no codigo:** `F2.1`.
- **Rejeitado apos reavaliacao:** `F5.5` (ver bloco do item — regrediria a latencia da busca SIMD em troca de ~30 MB de RSS).
- **Proximo lote recomendado:** continuar `F5.2` (varredura de hot paths restantes; batch parcial ja aplicado em `app/operations/message_handler/helpers.rs` e `infrastructure/directory_index.rs`).

---

## Como usar este plano

1. Trabalhe **um item por PR/commit**, na ordem das fases abaixo.
2. Em cada item siga o ciclo: **ler arquivo → aplicar mudança → `cargo build` → `cargo clippy` → teste manual rápido → commit**.
3. Não misture itens de fases diferentes no mesmo commit.
4. Se algo depender de uma abstração nova (ex.: `OwnedHandle`, `ComScope`), crie a abstração no **primeiro** item que precisa dela (Fase 0) e reuse depois.
5. Para cada item há uma seção **"Pronto quando"** — não feche o item antes disso.

---

## Fase 0 — Abstrações e utilitários de base

Estas abstrações desbloqueiam vários itens subsequentes. **Faça primeiro.**

### F0.1 — Wrapper RAII `OwnedHandle` [IMPLEMENTADO]
**Motivo:** elimina classes inteiras de leaks de `HANDLE` (C2, C3 e similares).
**Arquivo a criar:** `src/infrastructure/windows/owned_handle.rs`
**Status:** implementado em `src/infrastructure/windows/owned_handle.rs` e exportado por `src/infrastructure/windows/mod.rs`.
**Passos:**
1. Criar struct:
   ```rust
   use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

   pub struct OwnedHandle(HANDLE);

   impl OwnedHandle {
       pub fn new(h: HANDLE) -> Option<Self> {
           if h.is_invalid() || h == INVALID_HANDLE_VALUE { None } else { Some(Self(h)) }
       }
       pub fn as_raw(&self) -> HANDLE { self.0 }
       pub fn into_raw(self) -> HANDLE {
           let h = self.0;
           std::mem::forget(self);
           h
       }
   }

   impl Drop for OwnedHandle {
       fn drop(&mut self) {
           unsafe { let _ = CloseHandle(self.0); }
       }
   }

   unsafe impl Send for OwnedHandle {}
   unsafe impl Sync for OwnedHandle {}
   ```
2. Exportar via `pub mod owned_handle;` em `src/infrastructure/windows/mod.rs`.

**Pronto quando:** compila sozinho e passa `cargo clippy -- -D warnings`.
**Risco:** baixo (código novo, não toca nada existente).

---

### F0.2 — `ComScope` RAII para STA [IMPLEMENTADO]
**Motivo:** elimina S6/C crítico de COM sem apartment consistente.
**Arquivo a criar:** `src/infrastructure/windows/com_scope.rs`
**Status:** implementado em `src/infrastructure/windows/com_scope.rs`, com teste unitario basico. A abstracao ja foi adotada em `src/workers/file_operation_worker.rs`; a migracao dos outros `ComGuard` locais continua pendente nos itens especificos.
**Passos:**
1. Criar:
   ```rust
   use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

   pub struct ComScope { initialized: bool }

   impl ComScope {
       pub fn sta() -> Self {
           let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
           // S_OK ou RPC_E_CHANGED_MODE são falhas — só consideramos sucesso quando hr.is_ok()
           Self { initialized: hr.is_ok() }
       }
       pub fn is_initialized(&self) -> bool { self.initialized }
   }

   impl Drop for ComScope {
       fn drop(&mut self) {
           if self.initialized {
               unsafe { CoUninitialize(); }
           }
       }
   }
   ```
2. Documentar com `/// Deve ser criado e droppado na mesma thread.`

**Pronto quando:** compila, e existe pelo menos um teste unitário que chama `ComScope::sta()` e dropa.
**Risco:** baixo.

---

### F0.3 — Helper `spawn_named` [IMPLEMENTADO]
**Motivo:** padroniza tratamento de `Err` de `thread::spawn` e log de panics (Co4, C7).
**Arquivo a criar:** `src/infrastructure/threading.rs`
**Status:** implementado em `src/infrastructure/threading.rs` e exportado por `src/lib.rs`.
**Passos:**
1. Criar função:
   ```rust
   pub fn spawn_named<F, T>(name: &str, f: F) -> std::io::Result<std::thread::JoinHandle<Option<T>>>
   where
       F: FnOnce() -> T + Send + 'static,
       T: Send + 'static,
   {
       let name_owned = name.to_string();
       std::thread::Builder::new().name(name_owned.clone()).spawn(move || {
           let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
           match result {
               Ok(v) => Some(v),
               Err(payload) => {
                   log::error!(
                       "[{}] thread panicked: {}",
                       name_owned,
                       panic_message(&payload)
                   );
                   None
               }
           }
       })
   }

   fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
       if let Some(s) = payload.downcast_ref::<&'static str>() { (*s).to_string() }
       else if let Some(s) = payload.downcast_ref::<String>() { s.clone() }
       else { "<non-string panic payload>".to_string() }
   }
   ```
2. Expor em `src/lib.rs`.

**Pronto quando:** compila, `cargo clippy` limpo.
**Risco:** baixo.

---

## Fase 1 — Críticos (bloqueantes)

### F1.1 — C1: Buffer pequeno do `ReadDirectoryChangesW` e overflow silencioso [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs` (linha ~18)
**Status:** implementado com aumento do buffer para `256 * 1024`, deteccao de overflow/truncamento, emissao de `DriveWatcherEvent::PrefixInvalidated` e invalidacao do prefixo no consumidor de `user_session_search`.
**Passos:**
1. Alterar constante:
   ```rust
   const BUFFER_SIZE: usize = 256 * 1024; // 256 KB
   ```
2. Após cada `GetOverlappedResult` ou equivalente, detectar overflow:
   ```rust
   if bytes_returned == 0 {
       log::warn!("[DRIVE-WATCHER] ReadDirectoryChangesW overflow on prefix {:?}, invalidating", current_prefix);
       // enviar evento sintético de invalidação total do prefixo:
       let _ = event_tx.send(vec![DriveWatcherEvent::PrefixInvalidated(current_prefix.clone())]);
   }
   ```
3. Garantir que o receptor (`DriveWatcherEvent::PrefixInvalidated`) faça invalidação total do `DirectoryCache` para aquele prefixo.

**Pronto quando:**
- Teste manual: descompactar 10k arquivos em pasta observada. Não deve haver inconsistência visual no listing (F5 tem que mostrar o estado real).
- Log de warning aparece em bursts sintéticos (`copy /b NUL large.bin`).

**Risco de regressão:** **médio** — novo evento `PrefixInvalidated`. Verifique que o handler do evento já existe ou adicione-o.

---

### F1.2 — C2 + C3: Handle leaks no `drive_watcher` e `mft_reader` [IMPLEMENTADO]
**Arquivos:**
- `src/infrastructure/drive_watcher/thread_loop.rs`
- `crates/mtt-search-service/src/mft_reader.rs`

**Status:** implementado. O `drive_watcher` passou a encapsular o handle do diretorio e o evento overlapped com RAII (`OwnedHandle`), e o fallback `OpenFileById` do `mft_reader` passou a fechar o handle via guard local (`HandleGuard`) sem depender de `CloseHandle` manual em caminhos com retorno antecipado.

**Passos:**
1. Em `thread_loop.rs`:
   - Substituir `let h_event = CreateEventW(...)?` por:
     ```rust
     let h_event = OwnedHandle::new(unsafe { CreateEventW(None, true, false, None) }?)
         .ok_or_else(|| "CreateEventW returned invalid handle")?;
     ```
   - Trocar todo uso por `h_event.as_raw()`.
   - Remover `CloseHandle(h_event)` manual — o `Drop` faz isso.
   - Fazer o mesmo com o handle do diretório aberto via `CreateFileW`.
2. Em `mft_reader.rs`:
   - Localizar cada `CreateFileW` de volume, envolver em `OwnedHandle::new(...)`.
   - Eliminar `CloseHandle` manuais; reaproveitar `as_raw()` onde for preciso passar para Win32.
   - Cobrir especialmente o trecho de `GetFileSizeEx` (linha ~554-580) que tem early return.

**Pronto quando:**
- `cargo build --release` ok.
- Rodar aplicação por 10 minutos, abrir muitas pastas, ver em `Process Explorer` que o número de handles não cresce indefinidamente.

**Risco:** **médio** — handles viram `Drop`-based; um `mem::forget` involuntário causa leak, mas nada pior.

---

### F1.3 — C4: Eliminar `unwrap()` em parsing binário do MFT [IMPLEMENTADO]
**Arquivo:** `crates/mtt-search-service/src/mft_reader.rs`
**Status:** implementado. O arquivo agora usa helpers de leitura segura (`read_u16_le`, `read_u32_le`, `read_u64_le`, `read_i64_le`, `read_frn_le`, `read_attr_header`, `resident_attr_value_bounds`) e o `grep` por `unwrap()`/`try_into().unwrap()` no parser retorna zero.
**Passos:**
1. Criar helper local:
   ```rust
   fn read_u32_le(buf: &[u8], off: usize) -> Result<u32, String> {
       let end = off.checked_add(4).ok_or("offset overflow")?;
       if end > buf.len() {
           return Err(format!("read_u32_le: want {}..{}, have {}", off, end, buf.len()));
       }
       Ok(u32::from_le_bytes(buf[off..end].try_into().unwrap()))
       // o unwrap aqui é ok: já validamos o tamanho acima
   }
   // versão análoga para u16/u64
   ```
2. Substituir **todas** as ocorrências `u32::from_le_bytes(buf[a..b].try_into().unwrap())` por `read_u32_le(buf, a)?`.
3. Propagar `Result` na função chamadora (já existente? criar se não).
4. Mesma regra para `u16` e `u64`.
5. Testar com `cargo test -p mtt-search-service`.

**Pronto quando:**
- `grep -n "try_into().unwrap()" crates/mtt-search-service/src/mft_reader.rs` retorna zero.
- Testar manualmente: conectar pendrive formatado em exFAT (não é NTFS). O serviço não deve panicar — deve logar erro e seguir.

**Risco:** **alto** — mexe em todo o parser. Revisar com cuidado, testar antes de publicar.

---

### F1.4 — C5: `catch_unwind` no `file_operation_worker` [IMPLEMENTADO]
**Arquivo:** `src/workers/file_operation_worker.rs` (linhas ~257-275)
**Status:** implementado. O worker agora usa `ComScope`, e no caminho de panic envia `FileOperationResult::OperationFailed { message }` seguido de `Finished`, preservando a limpeza de estado da UI e exibindo erro ao usuario. Tambem foi adicionado log de falha no spawn do worker.
**Passos:**
1. Localizar o bloco `catch_unwind`. Hoje é:
   ```rust
   let result = std::panic::catch_unwind(...);
   ```
2. Após o bloco, adicionar:
   ```rust
   if let Err(payload) = result {
       let msg = if let Some(s) = payload.downcast_ref::<&'static str>() { (*s).to_string() }
                 else if let Some(s) = payload.downcast_ref::<String>() { s.clone() }
                 else { "file operation handler panicked".to_string() };
       log::error!("[FileOps] handler panic: {}", msg);
       let _ = result_sender.send(FileOperationResult::OperationFailed { message: msg });
       let _ = result_sender.send(FileOperationResult::Finished);
   }
   ```
3. Reutilizar a variant ja existente `FileOperationResult::OperationFailed { message: String }`; nao criar uma variant nova so para panics.

**Pronto quando:**
- Teste induzido: em debug, injete `panic!("injected")` em um handler por 1 run e confirme que a UI mostra mensagem de erro e libera o estado "operando".

**Risco:** **baixo**.

---

### F1.5 — C6: `h.join().unwrap()` em thumbnail worker [RECLASSIFICADO]
**Arquivo:** `src/workers/thumbnail/worker.rs` (linha ~329)
**Status:** a ocorrencia auditada esta em codigo de teste (`#[cfg(test)]`), nao no fluxo principal de runtime. Nao e um bloqueador de producao. Manter como limpeza opcional de testes, fora da fila critica.
**Passos:**
1. Substituir:
   ```rust
   h.join().unwrap();
   ```
   por:
   ```rust
   if let Err(payload) = h.join() {
       log::error!("[thumbnail] worker thread panicked: {:?}", payload);
   }
   ```
**Pronto quando:** opcional. Se ajustado, deve apenas manter a suite de testes limpa sem alterar o runtime.
**Risco:** **muito baixo**.

---

### F1.6 — C7: `HDEVNOTIFY` em `AtomicUsize` [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/windows/device_change.rs` (linhas ~20-30)
**Status:** implementado. O handle de notificacao agora fica em `Mutex<Option<DeviceNotificationHandle>>`, com unregister automatico em `Drop` e limpeza centralizada no shutdown/encerramento do loop.
**Passos:**
1. Substituir:
   ```rust
   static DEVICE_NOTIFICATION_HANDLE: AtomicUsize = AtomicUsize::new(0);
   ```
   por:
   ```rust
   use std::sync::Mutex;
   use windows::Win32::UI::WindowsAndMessaging::HDEVNOTIFY;

   struct NotifHandle(HDEVNOTIFY);
   unsafe impl Send for NotifHandle {}

   static DEVICE_NOTIFICATION_HANDLE: Mutex<Option<NotifHandle>> = Mutex::new(None);
   ```
2. Register:
   ```rust
   if let Ok(mut guard) = DEVICE_NOTIFICATION_HANDLE.lock() {
       *guard = Some(NotifHandle(h));
   }
   ```
3. Unregister (no shutdown):
   ```rust
   if let Ok(mut guard) = DEVICE_NOTIFICATION_HANDLE.lock() {
       if let Some(NotifHandle(h)) = guard.take() {
           unsafe { let _ = UnregisterDeviceNotification(h); }
       }
   }
   ```
**Pronto quando:** inserir/ejetar USB não vaza handle (visualizar com Process Explorer → View → Lower Pane → Handles).
**Risco:** **médio** — mudança de tipo global, revisar todos os `use` de `DEVICE_NOTIFICATION_HANDLE`.

---

## Fase 2 — Segurança / FFI

### F2.1 — S2: `GlobalLock` null check [JA IMPLEMENTADO]
**Arquivo:** `src/infrastructure/windows_clipboard.rs` (linhas ~138-200)
**Status:** ja esta coberto no codigo atual. `set_preferred_drop_effect()` valida `GlobalLock(hmem)` com `ptr.is_null()`, chama `GlobalFree` no erro e faz `GlobalUnlock` no sucesso; `get_preferred_drop_effect()` tambem verifica `ptr.is_null()` antes de ler.
**Passos:**
1. Em cada chamada `GlobalLock(...)`:
   ```rust
   let ptr = unsafe { GlobalLock(handle) };
   if ptr.is_null() {
       unsafe { let _ = GlobalFree(handle); }
       return Err("GlobalLock returned null".into());
   }
   ```
2. Adicionar `GlobalUnlock` no caminho de sucesso.
**Pronto quando:** ja atendido pelo codigo atual. Nao ha trabalho pendente neste item.
**Risco:** **baixo**.

---

### F2.2 — S4: `ImpersonateNamedPipeClient` tratamento preciso [IMPLEMENTADO]
**Arquivo:** `crates/mtt-search-service/src/ipc_authorization.rs` (linhas ~44-75)
**Status:** implementado com base no comportamento real da API. A documentacao oficial confirma que `ImpersonateNamedPipeClient` e uma API `BOOL`, nao `HRESULT`; no binding atual da crate `windows`, isso chega como `Result<()>` sem caminho `S_FALSE`. O guard agora marca `active` apenas no `Ok(())` e registra falha de `RevertToSelf()` no `Drop`.
**Passos:**
1. Ajustar o guard ao contrato real do binding:
   ```rust
   match unsafe { ImpersonateNamedPipeClient(pipe) } {
       Ok(()) => Ok(Self { active: true }),
       Err(e) => Err(format!("ImpersonateNamedPipeClient failed: {e}")),
   }
   ```
2. `Drop` só chama `RevertToSelf()` se `self.active`, e agora registra erro se o revert falhar.

**Pronto quando:** compila; teste manual de duas buscas simultâneas não quebra autorização.
**Risco:** **médio** — envolve segurança; revisar com cuidado.

---

### F2.3 — S5 + W4: ACL/SID do named pipe via APIs Win32 [IMPLEMENTADO]
**Arquivo:** `crates/mtt-search-service/src/ipc_server/pipe_io.rs` (linhas ~24-115)
**Status:** implementado. A montagem manual de bytes da ACL foi substituida por `AllocateAndInitializeSid` + `SetEntriesInAclW` + `InitializeSecurityDescriptor` + `SetSecurityDescriptorDacl`, com limpeza via RAII (`FreeSid` e `LocalFree`).
**Passos:**
1. Substituir montagem manual por `AllocateAndInitializeSid` + `SetEntriesInAclW` + `InitializeSecurityDescriptor` + `SetSecurityDescriptorDacl`.
2. Liberar com `FreeSid` e `LocalFree` nos caminhos de erro (usar guards RAII para isso).
3. Manter comportamento: `BUILTIN\Users` (S-1-5-32-545) read/write, `SYSTEM` (S-1-5-18) full control.

**Pronto quando:** pipe continua acessível para usuário comum; testar com `WinObj` que o descritor de segurança bateu.
**Risco:** **alto** — segurança do IPC. Revisar e testar em conta sem privilégios.

---

### F2.4 — S1: Asserts estáticos de layout [IMPLEMENTADO]
**Arquivo:** `crates/mtt-search-service/src/index_db/binary.rs` (linhas ~99, ~232, ~293)
**Status:** implementado. O modulo agora tem asserts de tamanho e `offset_of!` para `Header` e `FileRecord`, e os tamanhos do formato binario passaram a ser centralizados em constantes compartilhadas com os loops de serializacao/desserializacao.
**Passos:**
1. Marcar structs serializáveis com `#[repr(C)]` (ou `packed` se já for o caso).
2. Adicionar, logo após cada definição:
   ```rust
   const _: () = {
       assert!(std::mem::size_of::<Header>() == HEADER_SIZE);
       assert!(std::mem::size_of::<FileRecord>() == FILE_RECORD_SIZE);
   };
   ```
3. Fazer o mesmo para offsets críticos via `std::mem::offset_of!` (Rust 1.77+).
**Pronto quando:** compila. Se falhar, indica mismatch que precisa ser corrigido imediatamente.
**Risco:** **baixo** (se falhar, já é um bug real que a auditoria expôs).

---

### F2.5 — S3: Trocar `GetProcAddress(NtQueryDirectoryFile)` por bindings [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/ntfs_reader.rs` (linhas ~70-75)
**Status:** implementado. O codigo agora usa `windows::Wdk::Storage::FileSystem::NtQueryDirectoryFile` diretamente, com `IO_STATUS_BLOCK` oficial da crate `windows`. A feature `Wdk_Storage_FileSystem` foi habilitada no `Cargo.toml` do app principal, sem precisar introduzir `windows-sys` como dependencia separada.
**Passos:**
1. Habilitar a feature `Wdk_Storage_FileSystem` na dependencia `windows`.
2. Substituir `GetProcAddress + transmute` por `use windows::Wdk::Storage::FileSystem::NtQueryDirectoryFile;`.
3. Chamar diretamente.
**Pronto quando:** compila e `cargo test` passa; listagem de diretórios continua funcional.
**Risco:** **médio** — ABI precisa bater; se `windows-sys` não exportar, manter transmute mas adicionar assert de tamanho por enquanto.

---

### F2.6 — W5: Logar falha de `SetDefaultDllDirectories` [IMPLEMENTADO]
**Arquivos:** `src/main.rs`, `crates/mtt-search-service/src/main.rs`
**Status:** implementado. O binario principal agora usa `log::warn!` e o search service usa `eprintln!`, porque o crate do servico nao tinha logger inicializado nem dependencia de `log`.
**Passos:**
1. Trocar:
   ```rust
   let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
   ```
   por:
   ```rust
   if let Err(e) = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS) } {
       log::warn!("DLL search hardening failed: {} (process continues with reduced hardening)", e);
   }
   ```
**Pronto quando:** atendido. Falhas passam a ser visiveis em vez de silenciosamente descartadas.
**Risco:** **muito baixo**.

---

## Fase 3 — Windows API / I/O correto

### F3.1 — W1: `OVERLAPPED` reset entre chamadas [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs` (linhas ~64-65)
**Status:** implementado. Antes de cada nova chamada a `ReadDirectoryChangesW`, `overlapped` e zerado e o `hEvent` e restaurado, evitando reuso de estado residual entre leituras assincronas.
**Passos:**
1. Antes de cada `ReadDirectoryChangesW`:
   ```rust
   unsafe {
       let h = overlapped.hEvent;
       overlapped = std::mem::zeroed();
       overlapped.hEvent = h;
   }
   ```
**Pronto quando:** compila e watcher continua funcionando.
**Risco:** **baixo**.

---

### F3.2 — W2: Logar `GetLastError` após `DeviceIoControl` [IMPLEMENTADO]
**Arquivos:**
- `crates/mtt-search-service/src/mft_reader.rs`
- `crates/mtt-search-service/src/usn_journal.rs`

**Status:** implementado. As falhas de `DeviceIoControl` agora registram o `control_code`, o handle do volume e o `Win32 error` correspondente. No caminho de `FSCTL_GET_NTFS_FILE_RECORD`, `fetch_mft_record()` passou a usar `Result<Option<usize>, String>`, preservando retorno tipado para falha real do IOCTL e `None` apenas para casos nao resolvidos/records incompativeis.

**Passos:**
1. Em cada falha (`result.is_err()` ou equivalente):
   ```rust
   let err = unsafe { GetLastError() };
   log::error!(
       "DeviceIoControl({ctl:#x}) on {volume:?} failed: win32 error {}",
       err.0
   );
   ```
2. Substituir `return None` por retorno tipado quando possível (propagar código do erro para quem chama).

**Pronto quando:** forçar acesso negado (volume bloqueado) gera log com código específico (`5 = ERROR_ACCESS_DENIED`).
**Risco:** **baixo**.

---

### F3.3 — W3: Retirar syscalls Win32 da UI [IMPLEMENTADO]
**Arquivo:** `src/ui/app/lifecycle.rs` (linhas ~322-325)
**Status:** implementado com extracao para a camada `src/infrastructure/windows/`. Foi criado `process_snapshot.rs` para encapsular ToolHelp/`CancelSynchronousIo` e metricas de processo, e a UI passou a consumir helpers (`process_snapshot`, `key_state`, `window_focus`) em vez de chamar Win32 diretamente nos pontos auditados (`lifecycle`, `input`, `status_bar`, `video_preview/detached`). O fallback final em `main.rs` tambem foi alinhado para reutilizar os mesmos helpers.
**Passos:**
1. Criar `src/infrastructure/windows/process_snapshot.rs` contendo uma função `cancel_pending_io_on_current_process_threads()` que encapsula `CreateToolhelp32Snapshot`/`Thread32First`/`CancelSynchronousIo`.
2. No lifecycle da UI, chamar apenas essa função.
3. Mover qualquer outra chamada Win32 direta para o mesmo módulo.
**Pronto quando:** `grep -nE "unsafe \{ *(Create|Get)[A-Z]" src/ui/` retorna zero.
**Risco:** **baixo**.

---

## Fase 4 — Concorrência

### F4.1 — Co1: Cap do coalescing set antes do insert [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/drive_watcher/thread_loop.rs` (linhas ~120-180)
**Status:** implementado. O watcher agora drena e envia o lote atual antes de inserir novo evento quando `coalesced.len() >= MAX_COALESCED_EVENTS`, reduzindo o risco de crescimento excessivo do set em bursts.
**Passos:**
1. Antes do `coalesced.insert(event)`:
   ```rust
   if coalesced.len() >= MAX_COALESCED_EVENTS {
       let batch: Vec<_> = coalesced.drain().collect();
       let _ = event_tx.send(batch);
       last_flush = std::time::Instant::now();
   }
   coalesced.insert(event);
   ```
**Pronto quando:** descompactar 100k arquivos não faz a RSS do processo crescer em centenas de MB.
**Risco:** **baixo**.

---

### F4.2 — Co2: Migrar `Mutex` dos caches para `parking_lot::Mutex` [IMPLEMENTADO]
**Arquivos:**
- `src/infrastructure/directory_cache.rs`
- `src/infrastructure/directory_dirty_registry.rs`
- Qualquer outro cache com `.unwrap_or_else(|e| e.into_inner())`.

**Status:** implementado. O crate principal agora depende de `parking_lot`, e os caches/estruturas que ainda dependiam de poison recovery foram migrados para `parking_lot::Mutex`, incluindo `directory_cache`, `directory_dirty_registry`, caches auxiliares de `io_priority`, `file_type`, `codec_registry`, `file_flags`, `sidebar`, falhas de thumbnail, fila/semaforo de thumbnails e cache de GIF. O grep por `unwrap_or_else(|e| e.into_inner())` em `src/` agora retorna zero.

**Passos:**
1. Adicionar `parking_lot = "0.12"` em `Cargo.toml` se ainda não estiver.
2. Trocar `std::sync::Mutex` por `parking_lot::Mutex`.
3. Ajustar chamadas: `.lock()` retorna diretamente o guard (sem `Result`).
4. Remover `.unwrap_or_else(|e| e.into_inner())`.
5. Se a auditoria encontrou invariantes quebradas por panic, adicionar `clear()` preventivo apenas em estruturas que conseguirem se recuperar sem dados perdidos.

**Pronto quando:** `grep -n "unwrap_or_else(.e. e.into_inner())" src/` retorna zero (ou apenas em locais documentados).
**Risco:** **médio** — muitos arquivos. Fazer em PR dedicado.

---

### F4.3 — Co3: `notify_one()` fora do lock [IMPLEMENTADO]
**Arquivo:** `src/workers/thumbnail/queue.rs` (linhas ~330-340)
**Status:** implementado. A fila de thumbnails agora libera o `MutexGuard` antes de chamar `notify_one()`/`notify_all()`, evitando acordar a thread consumidora enquanto ainda segura o lock da fila.
**Passos:**
1. Antes:
   ```rust
   let mut state = self.state.lock()...;
   // ... update ...
   self.condvar.notify_one();
   ```
2. Depois:
   ```rust
   {
       let mut state = self.state.lock()...;
       // ... update ...
   } // drop do guard aqui
   self.condvar.notify_one();
   ```
**Pronto quando:** comportamento idêntico, sem regressões em `cargo test`.
**Risco:** **baixo**.

---

### F4.4 — Co4: Rollback de flags in-flight em `spawn()` falho [IMPLEMENTADO]
**Arquivos:**
- `crates/mtt-search-service/src/ipc_server/handler.rs` (linhas ~60-110)
- `src/workers/global_search_worker.rs` (linhas ~201-215)

**Status:** implementado. `global_search_worker` agora usa `spawn_named("global-search-total-count", ...)` com rollback explícito de `in_flight` quando o spawn falha, e o `WarmIndex` do search service passou a usar `std::thread::Builder::spawn(...)` com nome de thread, guard RAII para reset da flag e rollback imediato do `is_warming` no caminho de erro.

**Passos (padrão):**
```rust
if flag.swap(true, Ordering::AcqRel) { return; }
match std::thread::Builder::new().name("...".into()).spawn(move || {
    struct Reset<'a>(&'a AtomicBool);
    impl<'a> Drop for Reset<'a> {
        fn drop(&mut self) { self.0.store(false, Ordering::Release); }
    }
    let _guard = Reset(&flag_clone);
    // corpo
}) {
    Ok(_) => {}
    Err(e) => {
        log::error!("spawn failed: {e}");
        flag.store(false, Ordering::Release);
    }
}
```
**Pronto quando:** simular falha de spawn via stress test não paralisa o warming permanentemente.
**Risco:** **baixo**.

---

### F4.5 — Co5: Inicialização serializada do PDFium [IMPLEMENTADO]
**Arquivo:** `src/pdf_viewer/renderer.rs` (linhas ~100-120)
**Passos:**
1. Trocar `OnceCell<()>` por `OnceLock<Result<Pdfium, String>>` **ou** proteger com `std::sync::Once`.
2. Exemplo com `Once`:
   ```rust
   static PDFIUM_INIT: std::sync::Once = std::sync::Once::new();
   static PDFIUM_STATE: std::sync::Mutex<Option<Pdfium>> = std::sync::Mutex::new(None);

   pub(super) fn pdfium() -> Result<Pdfium, String> {
       PDFIUM_INIT.call_once(|| {
           if let Ok(p) = bind_pdfium() {
               *PDFIUM_STATE.lock().unwrap() = Some(p);
           }
       });
       PDFIUM_STATE.lock().unwrap().clone().ok_or_else(|| "pdfium not bound".into())
   }
   ```
**Pronto quando:** abrir 20 PDFs simultaneamente (tabs) não logra multiple binds — instrumentar `bind_pdfium` com contador atômico e confirmar == 1.
**Risco:** **médio**.

---

### F4.6 — Co6: Simplificar ordering no GC worker [IMPLEMENTADO]
**Arquivo:** `src/app/init_workers/background_jobs.rs`
**Passos:**
1. Se a flag é binária (start/stop) sem outros stores dependentes: usar `Relaxed` em ambos os lados:
   ```rust
   GC_WORKER_RUNNING.store(false, Ordering::Relaxed);
   while GC_WORKER_RUNNING.load(Ordering::Relaxed) { ... }
   ```
2. Se houver publicação de dados junto: manter `SeqCst` apenas onde há dependência.
**Pronto quando:** shutdown continua funcionando (< 1s até parar).
**Risco:** **baixo**.

---

### F4.7 — Co7: Acesso uniforme ao `current_prefix` do drive watcher [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/drive_watcher.rs` (linhas ~70-100)
**Passos:**
1. Decidir por **um** mecanismo:
   - **Opção A (recomendada):** adicionar `arc-swap = "1"` no `Cargo.toml`, trocar `Arc<Mutex<PathBuf>>` por `ArcSwap<PathBuf>`. Leituras sem lock.
   - **Opção B:** manter `Mutex`, mas garantir que **todas** as leituras/escritas usem o lock.
2. Remover o caminho que faz assignment direto em `current_prefix` na thread do watcher.
**Pronto quando:** mudança de prefixo durante navegação ativa não mistura eventos.
**Risco:** **médio**.

---

## Fase 5 — Performance

### F5.1 — P1: Passar `file_size` para `merge_video_metadata` [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/windows/metadata/video.rs` (linhas ~209-280)
**Passos:**
1. Adicionar parâmetro:
   ```rust
   pub fn merge_video_metadata(ps: MediaMetadata, mf: VideoMetadataMF, path: &Path, file_size: u64) -> MediaMetadata
   ```
2. Remover as duas chamadas internas a `std::fs::metadata(path)`.
3. Ajustar todos os callers para passar o `size` que já possuem (há pelo menos um em `folder_preview` / `folder_compose`).

**Pronto quando:** `cargo build` passa; abrir pasta com 1000 vídeos ficou mensuravelmente mais rápido (use `cargo bench` se disponível).
**Risco:** **baixo** (API interna).

---

### F5.2 — P2: Cortar `to_string_lossy().to_string()` em hot paths da UI [PARCIAL]

**Progresso (batch atual):** removido `.to_string()` redundante em `src/app/operations/message_handler/helpers.rs::normalize_for_match`/`clean_path` e em `src/infrastructure/directory_index.rs` (variavel `dir_str` agora usa `Cow<str>` direto). Restante da varredura ainda nao aplicado — a maioria das ocorrencias em `src/app/operations/drag_drop/rendering.rs`, `src/app/operations/tabs.rs`, `src/ui/app/panels/content.rs` e `src/ui/cache.rs` precisam permanecer `String` por causa do tipo de retorno de `unwrap_or_else` ou da chave de cache (`HashMap<String, _>`), entao o ganho real ali e zero sem refatorar a API a montante.

**Arquivos (amostra):**
- `src/image_viewer/mod.rs` (linhas ~209-217)
- `src/workers/thumbnail/progress.rs` (linha ~41)
- `src/tabs/mod.rs`
- `src/domain/file_entry.rs`

**Passos:**
1. Rodar:
   ```
   rg -n "to_string_lossy\(\)\.to_string\(\)" src/
   ```
2. Para cada ocorrência em loop/hot path, preferir:
   - Se já existe `entry.name: String`: usar `&entry.name`.
   - Se é `Path`: usar `path.to_string_lossy()` (Cow) e manter como `Cow`/`&str` até o último momento.
3. NÃO mudar ocorrências fora de hot path — foco é frame-rate UI e loops grandes.

**Pronto quando:** hot list de 1000 itens reduziu allocs/frame (inspecionar com `cargo flamegraph` ou `dhat` opcional).
**Risco:** **médio** (muitos arquivos). Fazer em commits pequenos.

---

### F5.3 — P3: Evicção O(log n) no cache de imagens [IMPLEMENTADO]
**Arquivo:** `src/image_viewer/cache.rs` (linhas ~57-73)
**Passos:**
1. Substituir `collect + sort` por:
   ```rust
   while self.total_bytes > MAX_CACHE_BYTES {
       let Some(&idx) = self.items.keys().max_by_key(|&&k| k.abs_diff(center)) else { break };
       if let Some(f) = self.items.remove(&idx) {
           self.total_bytes = self.total_bytes.saturating_sub(f.rgba.len());
       } else { break; }
   }
   ```
**Pronto quando:** rodar viewer em pasta com 500 imagens grandes; não há frame-drop visível na navegação.
**Risco:** **baixo**.

---

### F5.4 — P4: Lock por volume no indexador [IMPLEMENTADO]
**Arquivos:**
- NOVO: `crates/mtt-search-service/src/volume_indices.rs` — define `VolumeIndexHandle = Arc<RwLock<VolumeIndex>>` e `SharedVolumeIndices = Arc<RwLock<Vec<VolumeIndexHandle>>>`, com helpers `new_shared`, `handle_from`, `upsert`, `find_handle`, `snapshot_handles`. O upsert troca o conteudo via `*existing.write() = new_index;` para preservar handles ja distribuidos para threads em background (e.g. extracao de tamanhos).
- `crates/mtt-search-service/src/file_index.rs` — `search_page(handles: &[VolumeIndexHandle], ...)` agora trava cada volume individualmente via `handle.read()` na iteracao do loop externo.
- `crates/mtt-search-service/src/volume_indexers/usn.rs` e `non_usn.rs` — recebem `SharedVolumeIndices` e operam sobre o `VolumeIndexHandle` retornado por `volume_indices::upsert`. Threads de tamanho rodam com `bg_handle.try_write_for(...)` / `bg_handle.write()`, sem bloquear o lock externo.
- `crates/mtt-search-service/src/ipc_server/handler.rs` e `mod.rs`, `crates/mtt-search-service/src/ipc_authorization.rs` e `crates/mtt-search-service/src/main.rs` — propagam o novo tipo. WarmIndex / GetStatus / CheckPathsModified / FolderSize fazem `snapshot_handles` ou `find_handle` seguidos de `read()` por volume.

**Resultado:** o lock externo passa a guardar apenas o vetor de volumes (mutado raramente em discovery/upsert). Escritas pesadas (USN apply, prune, persist, refresh de tamanhos) ficam restritas ao volume alvo, permitindo que `search_page` continue lendo todos os outros volumes em paralelo.

**Validacao:** `cargo check --workspace` OK.
**Risco residual:** medio — a primeira atualizacao real de um indice depende do upsert sobrescrever no lugar (`*existing.write() = new_index;`), comportamento coberto por `volume_indices::upsert` e usado pelos dois indexadores.

---

### F5.4 (historico) — proposta original
**Arquivo:** `crates/mtt-search-service/src/volume_indexers/usn.rs` (linhas ~364-450)
**Passos:**
1. Trocar `RwLock<Vec<VolumeIndex>>` por `Vec<RwLock<VolumeIndex>>` (ou `DashMap<VolumeId, RwLock<...>>`).
2. Ajustar o indexer para travar **apenas** o volume que está escrevendo.
3. Ajustar queries que percorrem todos os volumes para fazer `read()` por volume individualmente.

**Pronto quando:** digitar na busca enquanto USN está reindexando volume grande não trava a UI.
**Risco:** **alto** — mudança estrutural. Dedicar PR isolado.

---

### F5.5 — P5: Lowercase lazy na `NameArena` [REJEITADO apos reavaliacao]
**Arquivo:** `crates/mtt-search-service/src/name_arena.rs` (linha ~110)

**Reavaliacao (Fase 5):** rejeitado. A `lowered` arena nao e overhead acidental —
e o que sustenta o caminho SIMD do `search_page`
(`crates/mtt-search-service/src/file_index.rs`, linhas ~655-680). Remove-la para
lowercase on-the-fly inverteria um trade-off ja otimizado em Phase 3.

**Numeros estimados (1.5 M arquivos, nome medio ~20 bytes):**

| Cenario | Custo de load (uma vez) | Custo por query | RSS |
|---|---|---|---|
| Atual (`build_lowered` no load) | clone 30 MB + `make_ascii_lowercase` (~50-100 ms) | 0 lowercase + `memchr::memmem` SIMD direto sobre slice ja-lowered (~10-50 ms para 1.7 M) | +30 MB |
| Proposto (lazy/per-record) | 0 | lowercase escalar de ~30 MB de bytes por query antes do SIMD (~30-80 ms extra por query) | -30 MB |

Resultado liquido: economiza 30 MB de RSS (irrisorio frente ao restante do indice),
mas **dobra ou triplica** a latencia de toda query de busca. Como Phase 3 trocou
FTS5 por SIMD em memoria justamente para alcancar sub-50 ms em 1.7 M arquivos,
regredir essa hot path nao se justifica.

**Alternativa avaliada e tambem rejeitada:** adiar `build_lowered()` para a
primeira query ou para uma thread de warmup. Apenas desloca a regressao da carga
inicial para a primeira busca do usuario, sem economia de RSS sustentada (a arena
acaba sendo construida do mesmo jeito).

**Pronto quando:** N/A — item fechado como nao-acionavel.
**Risco:** alto se reaberto (regressao de query latency mensuravel).

---

### F5.6 — P6: EXIF parsing em um único pass [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/windows/metadata/image.rs` (linhas ~32-68)
**Passos:**
1. Criar struct `ExifSummary { make, model, datetime, gps, ... }`.
2. Iterar `exifreader.fields()` uma vez preenchendo cada campo via `match field.tag`.
3. Retornar `ExifSummary` em vez de chamar `get_field` 15 vezes.
**Pronto quando:** folder com 5k imagens RAW: metadata read mensuravelmente mais rápido.
**Risco:** **baixo**.

---

### F5.7 — P7: Backoff exponencial nos retries [IMPLEMENTADO]
**Arquivos:**
- `src/app/init_workers/filesystem_workers.rs` (linha ~74)
- `src/image_viewer/ipc.rs` (linhas ~26, ~65)

**Passos:**
1. Padrão genérico:
   ```rust
   let mut delay = Duration::from_millis(10);
   loop {
       if try_op()? { break; }
       std::thread::sleep(delay);
       delay = (delay * 2).min(Duration::from_secs(2));
   }
   ```
2. Para IPC: usar `WaitForSingleObject(pipe_event, timeout)` quando possível.
**Pronto quando:** startup não desperdiça CPU em polling fixo.
**Risco:** **baixo**.

---

### F5.8 — P8: Prune de `pending_revalidation` [IMPLEMENTADO]
**Arquivo:** `src/app/folder_size_state.rs` (linhas ~96-107)
**Passos:**
1. Adicionar método:
   ```rust
   pub fn prune_expired_revalidations(&mut self) {
       let now = Instant::now();
       self.pending_revalidation.retain(|_, deadline| *deadline > now);
   }
   ```
2. Chamar periodicamente em `update()` quando o mapa tiver `> 500` entradas, ou a cada N frames.
**Pronto quando:** navegar em 2000 pastas diferentes não faz o mapa inflar permanentemente.
**Risco:** **baixo**.

---

### F5.9 — P9: Invalidação de prefixo no `DirectoryCache` [IMPLEMENTADO]
**Arquivo:** `src/infrastructure/directory_cache.rs` (linhas ~87-99)
**Passos:**
1. Substituir estrutura por `BTreeMap<PathBuf, CachedFolder>` OU manter LRU mas acrescentar índice auxiliar ordenado.
2. Implementar `invalidate_prefix(&Path)` que use `range(..)` para remover apenas a subárvore, sem varrer tudo.
**Pronto quando:** deletar pasta com 1000 subpastas cacheadas não degrada a UI.
**Risco:** **médio**.

---

## Fase 6 — Arquitetura

### F6.1 — A5: Tipos de erro consistentes
**Arquivos:** `src/domain/errors.rs` + `src/infrastructure/global_search.rs` + `src/application/file_operations.rs`
**Passos:**
1. Definir enums dedicados:
   ```rust
   #[derive(Debug, thiserror::Error)]
   pub enum SearchIpcError {
       #[error("pipe open failed: {0}")]
       PipeOpen(String),
       #[error("timeout after {0:?}")]
       Timeout(Duration),
       #[error("windows error {0}")]
       Win32(u32),
       #[error(transparent)]
       Io(#[from] std::io::Error),
   }
   ```
2. Substituir `Result<_, String>` progressivamente.
3. Preservar mensagens legíveis via `Display`.
**Pronto quando:** `rg "Result<.*, String>" src/infrastructure/ src/application/` retorna zero (ou apenas em pontos justificados).
**Risco:** **médio** (muitos callers).

---

### F6.2 — A1: Dividir `mft_reader.rs`
**Arquivo atual:** `crates/mtt-search-service/src/mft_reader.rs`
**Divisão alvo:**
- `mft_reader/mod.rs` — API pública + tipos.
- `mft_reader/geometry.rs` — leitura do boot sector (bytes_per_sector etc).
- `mft_reader/attributes.rs` — parsing de atributos.
- `mft_reader/record.rs` — parsing do file record.
- `mft_reader/volume.rs` — open/close do volume (usando `OwnedHandle`).

**Passos:**
1. Criar subpasta `mft_reader/` com `mod.rs` reexportando o que era público.
2. Mover funções por responsabilidade.
3. Rodar `cargo build -p mtt-search-service`.
**Pronto quando:** build ok, nenhum arquivo passa de ~400 linhas.
**Risco:** **médio** (muita movimentação).

---

### F6.3 — A1: Dividir `init_bootstrap.rs`
**Arquivo atual:** `src/app/init_bootstrap.rs`
**Divisão alvo:**
- `src/app/init/db.rs` — migração/abertura do SQLite.
- `src/app/init/channels.rs` — criação de canais.
- `src/app/init/workers.rs` — spawn inicial (complementa `init_workers/`).
- `src/app/init/state.rs` — composição do estado inicial.

**Passos:** mover por bloco lógico; garantir ordenação de dependências preservada.
**Pronto quando:** `init.rs` vira apenas um orquestrador (≤ 150 linhas).
**Risco:** **médio**.

---

### F6.4 — A1: Dividir `sidebar_tree_state.rs`
**Arquivo atual:** `src/app/state/sidebar_tree_state.rs`
**Divisão alvo:**
- `sidebar_tree/tree.rs` — estrutura e iteração.
- `sidebar_tree/navigation.rs` — lógica de expansão/seleção.
- `sidebar_tree/drag.rs` — drag-and-drop.

**Risco:** **médio**.

---

### F6.5 — A3: Barramento de invalidação de cache
**Novo arquivo:** `src/app/cache_bus.rs`
**Passos:**
1. Definir `pub enum CacheEvent { PathChanged(PathBuf), PrefixInvalidated(PathBuf), FullFlush }`.
2. Subscribers registram closures. Emitter substitui os múltiplos `pop()` espalhados em `helpers.rs`.
**Pronto quando:** `rg "\.pop\(" src/app/operations/message_handler/helpers.rs` mostra 1 linha (o dispatch do bus) em vez de 8.
**Risco:** **alto** (toca estado central). PR isolado.

---

### F6.6 — A4: Registry central de workers
**Novo arquivo:** `src/workers/registry.rs`
**Passos:**
1. `struct WorkerRegistry { handles: Vec<JoinHandle<()>>, stops: Vec<Arc<AtomicBool>> }`.
2. `fn register(&mut self, stop: Arc<AtomicBool>, handle: JoinHandle<()>)`.
3. `fn shutdown_all(&mut self, timeout: Duration)` — seta todos os stops, joina com timeout, loga quem ficou preso.
4. Migrar spawns de `app/init_workers/*.rs` e `workers/*.rs` para usar o registry.
**Pronto quando:** fechar a aplicação termina todas as threads em ≤ 2s ou loga worker preso.
**Risco:** **médio-alto**.

---

## Fase 7 — Verificação final

### F7.1 — Varredura automática
Rodar e garantir saída vazia (ou somente locais justificados):
```powershell
rg -n "\.unwrap\(\)" crates/mtt-search-service/src/mft_reader/
rg -n "try_into\(\)\.unwrap\(\)" crates/
rg -n "unwrap_or_else\(\|e\| e\.into_inner\(\)\)" src/
rg -n "to_string_lossy\(\)\.to_string\(\)" src/image_viewer/ src/workers/ src/tabs/ src/domain/
rg -n "std::thread::spawn\(" src/ crates/
rg -n "Result<.*, String>" src/infrastructure/ src/application/
```

### F7.2 — Testes de stress
1. **Cloud sync storm:** extrair 7z de 100k arquivos em pasta observada. Verificar:
   - Não há crash.
   - Cache não fica stale (F5 mostra estado real).
   - Memory usage estabiliza.
2. **Volume corrompido:** inserir pendrive com sistema de arquivos diferente de NTFS. Verificar que `mft_reader` loga erro e não derruba o serviço.
3. **High concurrency search:** disparar 20 buscas em < 1s. Verificar:
   - Nenhuma query fica travada.
   - Flag `in_flight` volta a `false`.
4. **COM / File ops:** copiar/mover 10k arquivos, incluindo pastas profundas. Verificar:
   - Nenhum handle vaza (Process Explorer).
   - UI recebe resposta mesmo se injetarmos panic no handler.

### F7.3 — Benchmarks
- Rodar `cargo bench` (existem benches em `benches/`).
- Comparar antes/depois para:
  - `image_viewer_decode`
  - `shell_ops_blocking`
  - Novos benches sugeridos: `folder_load_1m`, `search_query_hot`.

---

## Ordem de execução recomendada

| Ordem | Fase | Itens | Justificativa |
|------:|------|-------|---------------|
| 1 | 0 | F0.1, F0.2, F0.3 | Concluida |
| 2 | 1 | F1.1, F1.2, F1.3, F1.6 | Concluida (`F1.4` concluido, `F1.5` reclassificado) |
| 3 | 2 | F2.4, F2.2, F2.5, F2.3 | Concluida (`F2.1` ja implementado, `F2.6` concluido) |
| 4 | 3 | F3.2, F3.3 | Concluida (`F3.1` concluido) |
| 5 | 4 | F4.5, F4.6, F4.7 | Concluida (`F4.1`, `F4.2`, `F4.3` e `F4.4` concluidos anteriormente) |
| 6 | 5 | F5.2, F5.4 | Performance restante (`F5.1`, `F5.3`, `F5.6`, `F5.7`, `F5.8`, `F5.9` concluidos; `F5.5` rejeitado apos reavaliacao) |
| 7 | 6 | F6.1, F6.6, F6.5, F6.2, F6.3, F6.4 | Arquitetura por último |
| 8 | 7 | F7.1–F7.3 | Verificação |

---

## Regras de commit

- Prefixar commit com o ID do item, ex.:
  `F1.3: remove unwrap() in MFT parsing, return Result with context`
- Um item por commit. PRs agrupando itens da mesma fase são aceitáveis, mas **nunca** misture fases.
- Todo commit deve manter `cargo build --release` e `cargo clippy --all-targets -- -D warnings` verdes.
- Não documentar mudança em markdowns extras — este plano e o `AUDIT_REPORT.md` são as únicas referências.

---

## Riscos gerais e mitigação

| Risco | Mitigação |
|-------|-----------|
| Migração `std::sync::Mutex` → `parking_lot::Mutex` quebra locais sutis | PR isolado (F4.2); teste de stress dedicado |
| Split do `mft_reader` introduz regressão silenciosa | Rodar suíte de indexação em volume real antes e depois |
| Backoff exponencial pode atrasar startup em hardware lento | Cap do delay em ≤ 2s |
| `OwnedHandle` aplicado errado → duplo `CloseHandle` | Jamais chamar `CloseHandle` manual em handle owned; usar `into_raw()` quando precisar transferir |
| Mudança em tipos de erro quebra callers externos | Erros internos apenas; manter API pública estável se houver |

---

## Fim do plano

Ao completar as fases 0–7 com os critérios **"Pronto quando"** atendidos, todos os itens do `AUDIT_REPORT.md` estarão resolvidos ou explicitamente documentados como fora de escopo.
