# Audit Fix Implementation — Phased

## Fase 1 — Estabilidade: unwrap() seguro + Rename Validation (Quick Wins)
- [x] **STB-01**: Substituir [unwrap()](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/domain/errors.rs#166-171) perigoso em `recycle_bin.rs:158` por guard clause
- [x] **SEC-03**: Adicionar validação de nomes reservados do Windows no rename ([file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs))
- [x] **PERF-01/02**: Trocar `Lanczos3` → `CatmullRom` em `main.rs:10` e `disk_cache.rs:411`
- [x] Verificar compilação e testes existentes

## Fase 2 — Estabilidade: Mutex poisoning + COM guard
- [x] **STB-03**: ~~Substituir [lock().unwrap()](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/security.rs#499-506) por recovery~~ — já estava implementado com `unwrap_or_else`
- [x] **STB-04**: ~~Substituir `join().unwrap()`~~ — só existe em `#[cfg(test)]`, não em prod
- [x] **STB-08**: Usar [ComGuard](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/windows/shell_operations.rs#29-30) RAII em [file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs) + [ComMfGuard](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/thumbnail/worker.rs#100-104) em [worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/thumbnail/worker.rs)
- [x] Verificar compilação e testes existentes

## Fase 3 — Limpeza de código
- [x] **CODE-04**: Remover [temp_snippet_shell_ops.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/temp_snippet_shell_ops.rs) (arquivo temporário no repo)
- [x] **CODE-07**: Remover comentário duplicado em `state.rs:192-193`
- [x] Verificar compilação

## Fase 4 — Quick Wins Restantes
- [x] **STB-01**: Fix [unwrap()](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/domain/errors.rs#166-171) em `item_renderer.rs:171` → guard clause
- [x] **SEC-05**: Logar resultado do `icacls` em [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs)
- [x] **CODE-05**: [SendHwnd](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs#50-51) + `FileOperationRequest` → `pub(crate)` em [file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs)
- [~] **PERF-05**: ~~Cachear [operation_security_config()](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs#130-148)~~ — Pulado (OnceLock quebraria hot-plug de drives USB)
- [x] **CODE-02**: Remover dead code: [idle_warmup.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/idle_warmup.rs), [input.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/app/input.rs), [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs), [list_view/mod.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/views/list_view/mod.rs), [file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs)
- [x] Verificar compilação e testes

## Fase 5 — SEC-01: Hash Collisions (blake3)
- [x] Substituir `DefaultHasher` por `blake3` em [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs)

## Fase 6 — SEC-02/PERF-04: Logging Framework
- [ ] Substituir `eprintln!` por [log](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/windows/native_menu.rs#396-407) crate (338 ocorrências)

## Fase 7 — Refactors Grandes (sessão separada)
- [ ] STB-02, STB-05, CODE-01, CODE-06, SEC-04, STB-07, PERF-03/06/07, SEC-06, CODE-03
