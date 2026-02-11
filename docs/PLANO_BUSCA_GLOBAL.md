# Plano: Serviço de Busca Global via USN Journal

## Contexto

O MTT File Manager atualmente possui apenas busca local (filtro por nome na pasta atual, em `src/application/sorting.rs`). O objetivo é implementar uma **busca global** em todo o computador usando a **USN Journal API do NTFS**, que permite enumerar milhões de arquivos em segundos. Como a USN Journal requer privilégios de administrador, a solução é criar um **Windows Service** separado que roda como SYSTEM/admin e se comunica com o aplicativo principal via **Named Pipes**.

---

## Estrutura do Workspace

Projeto organizado como Cargo Workspace com 3 crates:

```
MTT-File-Manager-RUST/
  Cargo.toml                      # Workspace root + app principal
  crates/
    mtt-search-service/           # Serviço Windows
      Cargo.toml
      src/
        main.rs                   # Entry point + SCM integration
        usn_journal.rs            # Leitura da USN Journal (FSCTL_ENUM_USN_DATA, FSCTL_READ_USN_JOURNAL)
        file_index.rs             # Index in-memory: HashMap<u64, FileRecord>
        path_resolver.rs          # Reconstrução de path via parent reference chain
        index_db.rs               # Persistência SQLite do índice
        ipc_server.rs             # Named Pipe server
        service_control.rs        # Install/uninstall do serviço
    mtt-search-protocol/          # Tipos IPC compartilhados
      Cargo.toml
      src/
        lib.rs                    # SearchRequest, SearchResponse, serialização
```

---

## Fase 1: Serviço + IPC (CONCLUÍDA)

### 1. Protocolo IPC (`mtt-search-protocol/src/lib.rs`)

Tipos compartilhados entre serviço e app. Serialização via **bincode** (10-50x mais rápido que JSON). Framing: prefixo de 4 bytes (u32 LE) com o tamanho do payload.

- `SearchRequest`: Query, GetStatus, Ping
- `SearchResponse`: Results, Status, Pong, Error
- `SearchResultItem`: name, full_path, is_dir, size
- `IndexStatusInfo`: volumes (Vec<VolumeStatus>), total_files_indexed
- Funções `encode_message` / `decode_message` com length-prefix framing

### 2. USN Journal Core (`mtt-search-service/src/usn_journal.rs`)

**API calls:**

| Operação | IOCTL | Propósito |
|----------|-------|-----------|
| Query journal info | `FSCTL_QUERY_USN_JOURNAL` | Obtém journal_id e limites USN |
| Enumeração completa | `FSCTL_ENUM_USN_DATA` | Walk do MFT inteiro (scan inicial, 1-5s para milhões de arquivos) |
| Leitura incremental | `FSCTL_READ_USN_JOURNAL` | Mudanças desde último USN processado (loop a cada 2s) |

### 3. Índice In-Memory (`mtt-search-service/src/file_index.rs`)

- `FileRecord`: name, name_lower (pré-computado), parent_ref, is_dir, size
- `VolumeIndex`: HashMap<u64, FileRecord> (FRN -> record), last_usn, journal_id, state
- Busca: substring case-insensitive em `name_lower`, ~10-50ms para milhões de registros

### 4. Reconstrução de Path (`mtt-search-service/src/path_resolver.rs`)

- Walk da chain de parents no HashMap até root (FRN 5 = root NTFS)
- Limite de 256 níveis de profundidade

### 5. Persistência SQLite (`mtt-search-service/src/index_db.rs`)

- DB em `%PROGRAMDATA%\MTT-File-Manager\search_index.db`
- Tabelas: `volume_state` (journal_id, last_usn) + `file_records`
- Startup: load cached -> verify journal_id -> catch-up ou full re-scan
- Persist periódico a cada 5 minutos

### 6. Named Pipe Server (`mtt-search-service/src/ipc_server.rs`)

- NULL DACL para permitir conexões de usuários não-admin
- `PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE`
- Formato: `[4 bytes length LE][bincode payload]`

### 7. Windows Service (`mtt-search-service/src/main.rs` + `service_control.rs`)

```
mtt-search-service.exe install       # Instala como serviço AutoStart
mtt-search-service.exe uninstall     # Remove o serviço
mtt-search-service.exe run-console   # Debug mode
(sem args)                           # Dispatched pelo SCM
```

---

## Fase 2: UI + Integração com o App

### 1. Cliente IPC (`src/infrastructure/global_search.rs`)

- Conecta ao Named Pipe como cliente
- Funções: `search()`, `ping()`, `get_status()`

### 2. Worker Thread (`src/workers/global_search_worker.rs`)

- Segue padrão de `file_operation_worker.rs`
- Enums `GlobalSearchRequest` / `GlobalSearchResponse`
- Thread dedicada com mpsc channels

### 3. Estado no App (`src/app/state.rs`)

Novos campos:
- `global_search_sender/receiver`
- `global_search_query`, `global_search_results`
- `global_search_active`, `global_search_loading`, `global_search_available`

### 4. UI Overlay Modal (tipo Spotlight)

- Ativado via `Ctrl+Shift+F`
- Popup centralizado com campo de busca + lista de resultados
- Double-click navega para pasta contendo o arquivo
- Indicador de status do serviço (online/offline)
- Debounce no input (300ms)

---

## Arquivos Críticos de Referência

| Arquivo | Relevância |
|---------|-----------|
| `src/infrastructure/ntfs_reader.rs` | Padrão para Win32 API: CreateFileW, buffer parsing, structs repr(C) |
| `src/workers/file_operation_worker.rs` | Padrão para Request/Response enum, worker thread com mpsc |
| `src/infrastructure/drive_watcher.rs` | Padrão para monitoring thread com shutdown flag e event coalescing |
| `src/infrastructure/disk_cache.rs` | Padrão para SQLite com WAL mode |
| `src/ui/toolbar.rs` | Campo de busca existente |
| `src/app/state.rs` | Estado principal do app |
| `src/app/init.rs` | Inicialização de workers e channels |

---

## Verificação

1. **Build:** `cargo build --workspace` compila todos os 3 crates
2. **Console mode:** `mtt-search-service.exe run-console` (requer admin)
3. **Service mode:** `install` -> `sc start` -> query via Named Pipe
4. **App existente:** `cargo run -p mtt-file-manager` sem regressões
5. **Integração:** Ctrl+Shift+F abre overlay, busca retorna resultados do serviço
