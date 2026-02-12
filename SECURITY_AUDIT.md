# Relatório de Auditoria de Segurança -- MTT File Manager

**Data:** 2026-02-11
**Escopo:** Aplicativo principal + Search Service + Protocolo IPC

## Resumo Executivo

Auditoria completa de ~80+ módulos. Identificadas **42 vulnerabilidades** em 9 categorias:

| Severidade | Quantidade |
|------------|-----------|
| CRITICAL   | 5         |
| HIGH       | 11        |
| MEDIUM     | 16        |
| LOW        | 10        |

---

## VULNERABILIDADES CRITICAL

### C1. Proteção contra Symlinks/Junctions desativada em produção
- **Arquivos:** `src/application/file_operations.rs:43-51`, `src/workers/file_operation_worker.rs:141-151`
- **Problema:** Ambos os callers de `sanitize_path` configuram `allow_symlinks: true`, desativando completamente `check_symlink()`. Um atacante pode criar junctions (sem privilégio admin) para redirecionar operações de arquivo para locais arbitrários.
- **Impacto:** Copy/move/delete podem atingir diretórios do sistema.

### C2. Caminhos UNC ignoram toda validação de segurança
- **Arquivos:** `src/application/file_operations.rs:53-69`, `src/workers/file_operation_worker.rs:153-171`
- **Problema:** `should_bypass_sanitization()` retorna `true` para qualquer caminho `\\server\share`, pulando toda verificação de path traversal, drive, extensão e symlink.
- **Impacto:** Um caminho como `\\evil-server\share\..\..\sensitive` passa sem validação.

### C3. TOCTOU Race Condition em renomeação de arquivos
- **Arquivo:** `src/infrastructure/windows/shell_operations.rs:251-276`
- **Problema:** `new_path.exists()` é verificado, depois `SHFileOperationW` é chamado. Entre o check e o use, um symlink pode ser colocado no caminho.

### C4. TOCTOU Race Condition em criação de pastas
- **Arquivo:** `src/application/file_operations.rs:130-147`
- **Problema:** `fast_path_exists()` em loop seguido de `create_dir()`. Uma junction pode ser colocada entre o check e a criação.

### C5. TOCTOU Race Condition em criação de atalhos
- **Arquivo:** `src/application/file_operations.rs:202-242`
- **Problema:** Mesmo padrão check-then-act com `fast_path_exists()` seguido de `IPersistFile::Save()`.

---

## VULNERABILIDADES HIGH

### H1. DACL do Named Pipe concede FILE_CREATE_PIPE_INSTANCE a Users
- **Arquivo:** `crates/mtt-search-service/src/ipc_server.rs:243`
- **Problema:** Access mask `0x001F01FF` (FILE_ALL_ACCESS) permite que qualquer usuário crie instâncias do pipe, possibilitando pipe squatting (man-in-the-middle).

### H2. Bincode desserialização sem limite de tamanho (OOM crash)
- **Arquivo:** `crates/mtt-search-protocol/src/lib.rs:72`
- **Problema:** `bincode::deserialize()` com config default permite que um payload de 64KB declare strings de 4GB, causando crash por OOM no serviço Windows.

### H3. WebView2 sem configurações de segurança
- **Arquivo:** `src/pdf_viewer/webview.rs:320-363`
- **Problema:** DevTools, JavaScript, popups e scripts dialogs ficam habilitados (defaults do WebView2). PDF malicioso pode executar JavaScript.

### H4. WebView2 sem controle de navegação
- **Arquivo:** `src/pdf_viewer/webview.rs:320-363`
- **Problema:** Sem handlers `NavigationStarting` ou `NewWindowRequested`. Links em PDFs podem navegar para URLs arbitrárias (phishing, file://).

### H5. File URL sem sanitização nem encoding
- **Arquivo:** `src/pdf_viewer/window.rs:58`
- **Problema:** `format!("file:///{}", path.display())` sem percent-encoding. Caracteres como `#`, `%`, `?` quebram a URL. Sem validação de tipo de arquivo.

### H6. DLL Hijacking via WebView2Loader.dll
- **Arquivo:** `src/pdf_viewer/webview.rs:119-120, 461-463`
- **Problema:** `LoadLibraryW("WebView2Loader.dll")` sem caminho completo. DLL search order permite que DLL maliciosa no diretório da aplicação seja carregada.

### H7. validate_file_extension nunca é chamada
- **Arquivo:** `src/infrastructure/security.rs:320-336`
- **Problema:** A lista de extensões bloqueadas (.exe, .bat, .cmd, .ps1, .vbs, .js) é dead code. Nunca é invocada no pipeline de validação.

### H8. Drive whitelist permite todos os 26 drives (A-Z)
- **Arquivo:** `src/workers/file_operation_worker.rs:144`, `src/application/file_operations.rs:44`
- **Problema:** `('A'..='Z').map(...)` gera whitelist com todas as letras, anulando o controle.

### H9. Bypass de extensão via ponto final (file.exe.)
- **Arquivo:** `src/infrastructure/security.rs:321`
- **Problema:** `Path::extension()` retorna `None` para `file.exe.`, mas Windows remove o ponto e executa como `.exe`.

### H10. Fallback de restauração para C:\Users\Public\Desktop
- **Arquivo:** `src/app/operations/recycle_bin_ops.rs:34-48`
- **Problema:** Restauração da lixeira usa fallback hardcoded para diretório público, potencialmente expondo arquivos sensíveis.

### H11. Arquivo temporário previsível (symlink attack)
- **Arquivo:** `src/infrastructure/windows/native_menu.rs:194-201`
- **Problema:** `temp_dir().join("mtt_warmup_dummy.txt")` é previsível. Atacante pode pré-criar symlink, causando truncamento de arquivo alvo.

---

## VULNERABILIDADES MEDIUM

| ID | Arquivo | Linha(s) | Descrição |
|----|---------|----------|-----------|
| M1 | `ipc_server.rs` | 287 | Missing `PIPE_REJECT_REMOTE_CLIENTS` -- pipe acessível via rede |
| M2 | `ipc_server.rs` | 459,500 | Sem timeouts em ReadFile/WriteFile -- Slowloris DoS com 8 conexões |
| M3 | `ipc_server.rs` | 396-451 | Índice de filesystem completo queryable por qualquer processo local |
| M4 | `ipc_server.rs` | 402 | String slicing panic em boundary UTF-8 multi-byte |
| M5 | `index_db.rs` | 37-48 | Command injection via `cmd /C icacls` (requer controle de %PROGRAMDATA%) |
| M6 | `index_db.rs` | 44-48 | Falha silenciosa de hardening de permissões |
| M7 | `disk_cache.rs` | 743-745 | SQL injection latente em `execute_batch_delete` (table/key_col como &str) |
| M8 | `disk_cache.rs` | 31-35 | Diretório de cache sem hardening de permissões (cache poisoning) |
| M9 | `disk_cache.rs` | 561-572 | WebP decode sem verificação de integridade (CVE-2023-4863 risk) |
| M10 | `disk_cache.rs` | 255-258 | Hash 64-bit (SipHash) para cache keys -- colisão por birthday attack |
| M11 | `security.rs` | 70-118 | Sem detecção de NTFS Alternate Data Streams |
| M12 | `security.rs` | 70-118 | Sem bloqueio de nomes reservados (CON, NUL, PRN, COM1-9) |
| M13 | `global_search.rs` | 152-190 | Client não verifica identidade do server (pipe name squatting) |
| M14 | `mpv_preview/mod.rs` | 264-265 | `loadfile` sem validação de protocolo (http://, edl://) |
| M15 | `resize.rs:37`, `disk_cache.rs:352,587` | - | Integer overflow em `width * height * 4` sem checked_mul |
| M16 | `ntfs_reader.rs` | 130-144 | Buffer pointer sem bounds check contra tamanho do buffer |

---

## VULNERABILIDADES LOW

- Sem Unicode normalization em paths
- `to_string_lossy()` mascarando encoding inválido
- TOCTOU em canonicalization fallback
- WAL/SHM files sem proteção explícita
- Information leakage via stderr (paths, erros SQLite)
- Ausência de integridade em dados do SQLite index
- Integer truncation i64 para u64 sem bounds check
- `unwrap()` em Mutex/RwLock que pode crashar serviço
- `partial_cmp().unwrap()` com NaN possível
- Fallback path no `sanitize_path` pula re-validação

---

# PLANO DE CORREÇÃO

Todas as correções são cirúrgicas, sem alterar comportamento funcional, UI/UX ou performance.

## Fase 1 -- CRITICAL

### Fix C1+C2: Restaurar controles de segurança
- Mudar `allow_symlinks: false` nas SecurityConfig
- Adicionar whitelist de junctions conhecidas do Windows como exceção
- Aplicar validação de path traversal e extensão mesmo para UNC paths
- Restringir allowed_drives para drives realmente montados

### Fix C3+C4+C5: Eliminar TOCTOU race conditions
- Renomeação: Remover check exists() -- Shell API trata colisões
- Criação de pasta: Usar create_dir() + tratar AlreadyExists em loop
- Criação de atalho: Mesmo padrão baseado em falha

## Fase 2 -- HIGH

### Fix H1: Restringir access mask do Named Pipe
### Fix H2: Limitar bincode desserialização com with_limit()
### Fix H3+H4: Hardening do WebView2 (settings + navigation handlers)
### Fix H5: Percentencodar conversão path para URL
### Fix H6: Carregar DLL com caminho completo ou SetDllDirectoryW("")
### Fix H7: Integrar validate_file_extension nos entry points externos

> **Status:** Pendente -- requer análise de entry points antes da implementação.

A função `validate_file_extension` (security.rs) bloqueia extensões perigosas
(.exe, .bat, .cmd, .ps1, .vbs, .js) mas nunca é chamada em nenhum lugar do código.

**Por que NÃO integrar no pipeline geral (`sanitize_path` / `sanitize_operation_path`):**

O `sanitize_operation_path` é invocado em todas as operações de arquivo (copiar,
mover, deletar, renomear, criar atalho, restaurar da lixeira). Integrar o bloqueio
de extensões nesse ponto impediria o file manager de operar sobre executáveis e
scripts -- uma regressão funcional direta, já que gerenciar arquivos de qualquer
tipo é a função principal do aplicativo.

**Abordagem correta -- validação nos entry points de paths externos:**

O risco real é path injection: um path malicioso que entra no app por uma fonte
externa e resulta em execução de código sem que o usuário tenha escolhido
explicitamente o arquivo. A validação deve ser aplicada nos pontos onde paths
de fontes não confiáveis entram no sistema:

1. **Argumentos de linha de comando** -- paths passados via CLI (`--open`, etc.)
2. **Clipboard** -- paths colados na barra de endereço ou em diálogos
3. **Drag-and-drop externo** -- arquivos arrastados de outro aplicativo para o app
4. **IPC / protocol handlers** -- paths recebidos de outros processos
5. **Ações automáticas** -- qualquer fluxo que execute um arquivo sem interação
   explícita do usuário (e.g., abrir arquivo passado por argumento)

Ações manuais do usuário sobre arquivos já listados na UI (duplo-clique,
menu de contexto) NÃO precisam dessa validação, pois o usuário vê o arquivo
e decide conscientemente interagir com ele.

**Passos para implementação:**

1. Mapear todos os entry points de paths externos no código
2. Criar uma função de sanitização específica para esses entry points que
   inclua `validate_file_extension` além das validações existentes
3. Manter `sanitize_operation_path` inalterado para operações internas
4. Avaliar se a lista de extensões bloqueadas deve ser configurável pelo usuário
### Fix H8: Detectar drives montados via GetLogicalDrives()
### Fix H9: Normalizar trailing dots/spaces antes de checar extensão
### Fix H10: Retornar erro em vez de fallback para Public Desktop
### Fix H11: Usar nome temporário único com PID

## Fase 3 -- MEDIUM

### Fix M1: Adicionar PIPE_REJECT_REMOTE_CLIENTS
### Fix M2: Timeouts em I/O do pipe (overlapped + 30s)
### Fix M4: Truncação UTF-8 segura com floor_char_boundary
### Fix M5: Chamar icacls diretamente sem cmd /C
### Fix M7: Enum para nomes de tabela em execute_batch_delete
### Fix M8: ACL no diretório de cache de thumbnails
### Fix M9: CRC32 nos BLOBs antes de decode WebP
### Fix M11+M12: Detecção de ADS e nomes reservados
### Fix M15: checked_mul para cálculos de tamanho
### Fix M16: Bounds check em ntfs_reader.rs

## Fase 4 -- LOW

- Substituir unwrap() em Mutex/RwLock por recovery
- unwrap_or(Ordering::Equal) em partial_cmp
- Guards em logs que expõem caminhos
- Normalização Unicode em paths (NFC)
