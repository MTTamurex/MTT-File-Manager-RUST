# Audit Fix Implementation Plan — Phased

Implementação incremental dos fixes identificados na auditoria. Cada fase é independente e será confirmada pelo usuário antes de prosseguir.

---

## Fase 1 — Quick Wins: Estabilidade + Segurança + Performance

### Estabilidade

#### [MODIFY] [recycle_bin.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/windows/recycle_bin.rs)

**STB-01: `unwrap()` perigoso na linha 158**

`EnumObjects` pode retornar `Ok(())` mas `enum_list_opt` ser `None` (se a Lixeira estiver vazia ou o objeto não suportar enumeração). O `unwrap()` causa panic.

```diff
-        let enum_list = enum_list_opt.unwrap();
+        let enum_list = match enum_list_opt {
+            Some(list) => list,
+            None => {
+                let _ = sender.send(Vec::new());
+                return;
+            }
+        };
```

### Segurança

#### [MODIFY] [file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs)

**SEC-03: Rename validation — reutilizar `is_windows_reserved_name` e adicionar chars ilegais**

Linhas 249-254: adicionar validação de caracteres ilegais (`<>:"|?*`) e nomes reservados do Windows. A função `is_windows_reserved_name` já existe em `security.rs` (linha 287).

```diff
+use crate::infrastructure::security::is_windows_reserved_name;
 ...
-                    if new_name.contains('\0')
-                        || new_name.contains('\\')
-                        || new_name.contains('/')
-                        || new_name == "."
-                        || new_name == ".."
+                    let invalid_chars = new_name.contains('\0')
+                        || new_name.contains('\\')
+                        || new_name.contains('/')
+                        || new_name.contains('<')
+                        || new_name.contains('>')
+                        || new_name.contains(':')
+                        || new_name.contains('"')
+                        || new_name.contains('|')
+                        || new_name.contains('?')
+                        || new_name.contains('*');
+                    let base_name = new_name.split('.').next().unwrap_or("");
+                    if invalid_chars
+                        || new_name == "."
+                        || new_name == ".."
+                        || new_name.ends_with('.')
+                        || new_name.ends_with(' ')
+                        || is_windows_reserved_name(base_name)
```

> [!IMPORTANT]
> Preciso tornar `is_windows_reserved_name` pública em `security.rs` (atualmente é `fn`, não `pub fn`).

### Performance

#### [MODIFY] [main.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/main.rs)

**PERF-01: Trocar `Lanczos3` → `CatmullRom` no resize do ícone de startup**

Linha 10: Para um ícone de janela 256×256, `CatmullRom` é ~3x mais rápido com qualidade visual idêntica.

```diff
-            let resized = img.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
+            let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
```

#### [MODIFY] [disk_cache.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/infrastructure/disk_cache.rs)

**PERF-02: Trocar `Lanczos3` → `CatmullRom` no resize de thumbnails para cache**

Linha 411: Thumbnails são exibidos em 64-512px, resize de 1024px não precisa de Lanczos3.

```diff
-            dynamic_img.resize(1024, 1024, image::imageops::FilterType::Lanczos3)
+            dynamic_img.resize(1024, 1024, image::imageops::FilterType::CatmullRom)
```

---

## Fase 2 — Concorrência: Mutex poisoning + COM Guard

#### [MODIFY] [worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/thumbnail/worker.rs)

**STB-03: Mutex poisoning recovery**

Substituir `lock().unwrap()` por recovery pattern que ignora o poisoned state.

**STB-04: Thread join com recovery**

Substituir `h.join().unwrap()` por `unwrap_or_else` com log de erro.

#### [MODIFY] [file_operation_worker.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/workers/file_operation_worker.rs)

**STB-08: COM guard RAII pattern**

Reutilizar o pattern `ComGuard` (já existe em `shell_operations.rs` e `recycle_bin.rs`) para garantir `CoUninitialize` mesmo em caso de panic.

---

## Fase 3 — Limpeza de Código

#### [DELETE] [temp_snippet_shell_ops.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/temp_snippet_shell_ops.rs)

**CODE-04: Arquivo temporário na raiz do repositório.**

#### [MODIFY] [state.rs](file:///c:/Users/mtamu/github/MTT-File-Manager-RUST/src/app/state.rs)

**CODE-07: Comentário duplicado na linha 192-193.**

```diff
-    // CLIPBOARD (Copiar/Recortar/Colar)
     // CLIPBOARD (Copiar/Recortar/Colar)
```

---

## Verification Plan

### Automated Tests

```powershell
# Compilar o projeto (valida que nenhuma mudança quebrou a build)
cargo build 2>&1

# Rodar testes existentes (27 módulos #[cfg(test)])
cargo test 2>&1

# Testes específicos de segurança (verifica rename validation com nomes reservados)
cargo test --lib infrastructure::security 2>&1
```

### Manual Verification

> [!NOTE]
> Após compilação e testes passando, pedir ao usuário para executar o app e verificar:
> 1. Abrir a Lixeira vazia (testa o fix do `unwrap` em `recycle_bin.rs`)
> 2. Renomear um arquivo para "CON" ou "NUL" (deve ser bloqueado pelo novo check)
> 3. Verificar que thumbnails ainda são gerados normalmente (testa o swap CatmullRom)
