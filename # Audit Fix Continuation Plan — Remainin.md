# Audit Fix Continuation Plan — Remaining Items

9 de 28 itens já foram implementados (Fases 1-3). Este plano cobre os 19 restantes.

> [!IMPORTANT]
> Itens marcados com **⚠️ REFACTOR GRANDE** requerem decisão do usuário sobre escopo e prioridade, pois tocam muitos arquivos e podem introduzir regressões.

---

## Fase 4 — Quick Wins Restantes

Mudanças isoladas, baixo risco, sem dependências.

### [MODIFY] [item_renderer.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/views/list_view/item_renderer.rs)
**STB-01 (restante):** `renaming_state.as_ref().unwrap()` na linha 171 — substituir por guard clause.

### [MODIFY] [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs)
**SEC-05:** Logar resultado de `icacls` em vez de silenciar com `let _`.

### [MODIFY] [file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs)
**CODE-05:** Trocar `pub` de [SendHwnd](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs#50-51) → `pub(crate)`.
**PERF-05:** Cachear [operation_security_config()](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs#143-161) em `OnceLock`.

### Dead code cleanup (CODE-02)
Remover `#[allow(dead_code)]` e o código morto nos arquivos:
- [idle_warmup.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/idle_warmup.rs) (3 ocorrências)
- [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs) (2)
- `list_view/mod.rs` (2)
- [input.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/ui/app/input.rs) (1)

### Verificação de STB-06
`folder_size.rs:144` — `sub_dirs.into_iter().next().unwrap()` está dentro de `1 =>` match arm, portanto é **seguro**. Nenhuma mudança necessária.

---

## Fase 5 — SEC-01: Hash Collisions no Cache

### [MODIFY] [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs)

Substituir `DefaultHasher` (64-bit, instável entre versões) por hash estável e resistente a colisões.

**Opção A:** Usar `path + modified_time` como chave composta (a coluna `modified_at` já existe).
**Opção B:** Usar hash 128-bit estável (ex: `FxHasher` já no projeto ou `blake3`).

> [!NOTE]
> Preciso da preferência do usuário: **Opção A** (sem nova dependência) ou **Opção B** (hash mais forte)?

---

## Fase 6 — SEC-02 / PERF-04: Logging Framework

### Afetado: ~338 chamadas `eprintln!` no projeto inteiro

**Abordagem proposta:**
1. Adicionar [log](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/windows/native_menu.rs#396-407) crate (já é padrão em Rust)
2. Substituir `eprintln!` → `log::debug!` / `log::warn!` / `log::error!`
3. Em release builds, configurar nível mínimo como `warn`
4. Hot paths (thumbnail stages) → `log::trace!`

> [!WARNING]
> Esta mudança toca ~338 locais. Recomendo fazer por módulo com confirmação entre cada.

---

## Fase 7 — Refactors Grandes (requer decisão do usuário)

Estes itens são significativos e requerem planejamento detalhado:

| ID | Descrição | Esforço |
|----|-----------|---------|
| **STB-02** | God-struct: agrupar 100+ campos de [ImageViewerApp](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/state.rs#66-365) em sub-structs | ⚠️ Alto |
| **STB-05** | Split [init.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/init.rs) (907 linhas) em funções `init_*` | ⚠️ Alto |
| **CODE-01** | 20+ funções com too_many_arguments → context structs | ⚠️ Alto |
| **CODE-06** | Renomear [ImageViewerApp](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/state.rs#66-365) → `FileManagerApp` | Médio (grep/replace global) |
| **SEC-04** | Named Pipe IPC authentication | Médio (design necessário) |
| **STB-07** | PDF viewer: raw vtable → crate `webview2-com` | Médio (nova dependência) |
| **PERF-03** | Dynamic SQL → prepared statement caching | Baixo-Médio |
| **PERF-06** | Named pipe polling → overlapped I/O | Médio |
| **PERF-07** | `to_string_lossy().to_string()` cleanup | Baixo (espalhado) |
| **SEC-06** | WebP dimension limits | Baixo |
| **CODE-03** | Padronizar idioma dos comentários | Baixo (espalhado) |

---

## Verification Plan

```powershell
cargo build 2>&1
cargo test 2>&1
```

Testes manuais específicos serão definidos por fase.
