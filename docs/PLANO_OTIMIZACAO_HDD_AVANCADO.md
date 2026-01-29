# Plano de Implementacao Avancado - Otimizacoes HDD Mecanico

**Data:** 2026-01-29
**Projeto:** MTT File Manager
**Pre-requisitos:** Implementacoes do `PLANO_OTIMIZACAO_HDD.md` ja realizadas

---

## Analise de Compatibilidade

### Estado Atual do Sistema

| Componente | Implementacao Atual | Arquivo |
|------------|---------------------|---------|
| Thread Priority | `THREAD_MODE_BACKGROUND_BEGIN` | `io_priority.rs:183-189` |
| FileSystem Watcher | Crate `notify` | `app/operations/watcher.rs` |
| I/O Pattern | Threads sincronas bloqueantes | `thumbnail_worker.rs` |
| Deteccao SSD/HDD | `IOCTL_STORAGE_QUERY_PROPERTY` | `io_priority.rs:76-161` |

### Matriz de Compatibilidade entre Otimizacoes

```
                        | Sequential | I/O Prio | USN Journal | Overlapped | MFT Read |
------------------------|------------|----------|-------------|------------|----------|
FILE_FLAG_SEQUENTIAL    |     -      |    OK    |     OK      |     OK     |   N/A    |
I/O Priority (kernel)   |    OK      |     -    |     OK      |   CONFLITO |   N/A    |
USN Journal             |    OK      |    OK    |      -      |     OK     |   OK     |
Overlapped I/O          |    OK      | CONFLITO |     OK      |      -     |   N/A    |
MFT Direct Read         |   N/A      |   N/A    |     OK      |    N/A     |    -     |
```

### Conflitos Identificados e Decisoes

#### 1. Thread Priority vs I/O Priority por Handle

| Aspecto | Thread Priority (Atual) | I/O Priority Handle |
|---------|------------------------|---------------------|
| Escopo | Toda a thread | Arquivo especifico |
| API | `THREAD_MODE_BACKGROUND_BEGIN` | `SetFileInformationByHandle` |
| Granularidade | Baixa | Alta |

**Analise:** NAO sao conflitantes - sao COMPLEMENTARES.
- Thread priority define a prioridade BASE
- Handle priority pode SOBRESCREVER para operacoes especificas

**Cenario util:** Thread em modo background, mas um arquivo especifico (clicado pelo usuario) precisa de prioridade normal.

**Decisao:** ✅ IMPLEMENTAR AMBOS. I/O Priority por Handle complementa o sistema existente.

---

#### 2. Thread Priority vs Overlapped I/O

| Aspecto | Thread Priority (Atual) | Overlapped I/O |
|---------|------------------------|----------------|
| Modelo | 4 threads sincronas | 1 thread + completion ports |
| Complexidade | Baixa | Alta |
| Refatoracao | Nenhuma | Arquitetura completa |
| Ganho em HDD | Medio | Baixo (seek e o gargalo) |

**Analise:** Sao CONFLITANTES.
- Thread priority nao faz sentido com I/O assincrono
- O gargalo real (seek do HDD) nao e resolvido por Overlapped I/O
- Refatoracao massiva necessaria

**Decisao:** ❌ NAO IMPLEMENTAR Overlapped I/O. Custo alto, beneficio baixo para o caso de uso.

---

#### 3. USN Journal vs notify (FileSystemWatcher)

| Aspecto | notify (Atual) | USN Journal |
|---------|----------------|-------------|
| Overhead | Alto (callbacks por evento) | Baixo (polling batch) |
| Confiabilidade | Pode perder eventos | Nunca perde |
| Multi-drive | Precisa de watcher por pasta | Um leitor por volume |
| Filesystem | Qualquer | NTFS only |

**Analise:** USN Journal e SUPERIOR em todos os aspectos para NTFS.

**Decisao:** ✅ USN Journal SUBSTITUI notify (com fallback para non-NTFS).

---

#### 4. MFT Direct Read vs NtQueryDirectoryFile

| Aspecto | NtQueryDirectoryFile (Implementado) | MFT Direct Read |
|---------|-------------------------------------|-----------------|
| Velocidade | Muito rapido | Extremamente rapido |
| Privilegios | Usuario normal | **ADMINISTRADOR** |
| Escopo | Uma pasta por vez | **Volume inteiro** |
| Complexidade | Media | Muito alta |
| Caso de uso | Navegacao de pastas | Indexacao global |

**Analise:** MFT Direct Read e SUPERIOR para indexacao global, mas requer admin.

**Decisao:** ✅ IMPLEMENTAR como OPCIONAL (Fase 5) para funcionalidades futuras como busca global.

---

## Otimizacoes Aprovadas para Implementacao

### Ordem de Implementacao (por impacto e dependencias)

```
Fase 1: FILE_FLAG_SEQUENTIAL_SCAN     [Sem dependencias, baixo risco]
Fase 2: Persistent Thumbnail Index    [Sem dependencias, baixo risco]
Fase 3: USN Journal                   [Substitui notify, medio risco]
Fase 4: I/O Priority por Handle       [Complementa thread priority]
Fase 5: MFT Direct Read               [OPCIONAL - NAO IMPLEMENTAR SEM ORDEM EXPLICITA]
```

> **⚠️ IMPORTANTE:** A Fase 5 (MFT Direct Read) NAO deve ser implementada automaticamente.
> Aguardar solicitacao explicita do usuario antes de iniciar esta fase.
> Motivos: requer privilegios de administrador, alta complexidade, uso especifico.

### Resumo das Decisoes de Conflitos

| Conflito | Decisao | Motivo |
|----------|---------|--------|
| Thread Priority + I/O Handle Priority | ✅ Manter ambos | Complementares |
| Thread Priority + Overlapped I/O | ❌ Manter Thread Priority | Overlapped nao resolve seek |
| notify + USN Journal | ✅ Migrar para USN | USN e superior para NTFS |
| NtQueryDirectoryFile + MFT Read | ✅ Manter NtQuery + MFT opcional | MFT para busca global |

---

## Fase 1: FILE_FLAG_SEQUENTIAL_SCAN

### 1.1 Descricao
Ao abrir arquivos para leitura sequencial (thumbnails, imagens), usar a flag `FILE_FLAG_SEQUENTIAL_SCAN` que instrui o Windows a otimizar o read-ahead do cache de disco.

### 1.2 Impacto
- **Beneficio:** ~15-30% mais rapido em leituras sequenciais de arquivos grandes
- **Risco:** Nenhum (flag puramente informativa)
- **Compatibilidade:** Funciona com todas as outras otimizacoes

### 1.3 Arquivos a Modificar

#### 1.3.1 Criar Helper: `src/infrastructure/windows/file_flags.rs`

```rust
//! Optimized file opening with appropriate flags for different access patterns
//!
//! This module provides wrappers around CreateFileW that automatically select
//! optimal flags based on the intended access pattern.

use std::fs::File;
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_SEQUENTIAL_SCAN, FILE_FLAG_RANDOM_ACCESS,
    FILE_GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING,
};

/// Open a file optimized for sequential reading (thumbnails, images)
///
/// Uses FILE_FLAG_SEQUENTIAL_SCAN which tells Windows to:
/// - Prefetch data ahead of reads
/// - Not cache data that's already been read (it won't be re-read)
///
/// This significantly improves performance on HDDs for sequential file access.
pub fn open_sequential(path: &Path) -> std::io::Result<File> {
    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_SEQUENTIAL_SCAN,
            None,
        )
    };

    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => {
            // SAFETY: We just created a valid file handle
            Ok(unsafe { File::from_raw_handle(h.0 as *mut std::ffi::c_void) })
        }
        Ok(_) => Err(std::io::Error::last_os_error()),
        Err(e) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        )),
    }
}

/// Open a file optimized for random access (seeking within file)
///
/// Uses FILE_FLAG_RANDOM_ACCESS which tells Windows to:
/// - Not prefetch data (reads are unpredictable)
/// - Keep all read data in cache (might be re-read)
///
/// Use this for video seeking or when reading specific file sections.
pub fn open_random_access(path: &Path) -> std::io::Result<File> {
    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAG_RANDOM_ACCESS,
            None,
        )
    };

    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => {
            Ok(unsafe { File::from_raw_handle(h.0 as *mut std::ffi::c_void) })
        }
        Ok(_) => Err(std::io::Error::last_os_error()),
        Err(e) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn test_open_sequential() {
        // Test with a known system file
        let result = open_sequential(Path::new("C:\\Windows\\System32\\ntdll.dll"));
        assert!(result.is_ok());

        let mut file = result.unwrap();
        let mut buffer = [0u8; 2];
        assert!(file.read(&mut buffer).is_ok());
        assert_eq!(&buffer, b"MZ"); // PE header magic
    }

    #[test]
    fn test_open_random_access() {
        let result = open_random_access(Path::new("C:\\Windows\\System32\\ntdll.dll"));
        assert!(result.is_ok());
    }
}
```

#### 1.3.2 Registrar Modulo

Em `src/infrastructure/windows/mod.rs`, adicionar:

```rust
pub mod file_flags;
```

#### 1.3.3 Usar em `thumbnail_worker.rs`

Modificar a funcao `try_image_crate_extraction` em `src/workers/thumbnail_worker.rs`:

```rust
// ANTES (linha ~642):
fn try_image_crate_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    // ...
    match image::open(path) {
        // ...
    }
}

// DEPOIS:
fn try_image_crate_extraction(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    if !matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "bmp" | "gif" | "webp" | "tiff"
    ) {
        return None;
    }

    // OPTIMIZATION: Use sequential scan flag for HDD optimization
    // This tells Windows to prefetch data ahead of our reads
    use crate::infrastructure::windows::file_flags::open_sequential;
    use std::io::BufReader;

    let file = open_sequential(path).ok()?;
    let reader = BufReader::with_capacity(65536, file); // 64KB buffer

    // Use image crate's reader interface instead of path
    let format = image::ImageFormat::from_extension(&ext)?;
    match image::load(reader, format) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            Some((rgba.to_vec(), rgba.width(), rgba.height()))
        }
        Err(_) => None,
    }
}
```

#### 1.3.4 Usar em WIC Extraction

Modificar `try_wic_extraction` para usar a nova funcao quando possivel (WIC usa seu proprio CreateFileW internamente, entao essa otimizacao e mais para imagem crate).

---

## Fase 2: Persistent Thumbnail Index

### 2.1 Descricao
Manter um indice SQLite de quais arquivos existem em cada pasta, junto com seus timestamps de modificacao. Isso permite:
- Detectar rapidamente se o cache de thumbnails esta atualizado
- Evitar re-scan completo de pastas
- Invalidar apenas thumbnails de arquivos modificados

### 2.2 Impacto
- **Beneficio:** ~40-60% menos I/O ao revisitar pastas
- **Risco:** Baixo (complementa cache existente)
- **Compatibilidade:** Funciona com todas as otimizacoes

### 2.3 Implementacao Detalhada

#### 2.3.1 Adicionar Tabelas no SQLite

Modificar `src/infrastructure/disk_cache.rs`:

```rust
// Adicionar na funcao init_db() apos as tabelas existentes:

// Directory index for fast change detection
conn.execute(
    "CREATE TABLE IF NOT EXISTS directory_index (
        dir_path TEXT PRIMARY KEY,
        file_count INTEGER NOT NULL,
        total_size INTEGER NOT NULL,
        last_scan_time INTEGER NOT NULL,
        scan_duration_ms INTEGER NOT NULL
    )",
    [],
)?;

// File entries for each indexed directory
conn.execute(
    "CREATE TABLE IF NOT EXISTS file_index (
        id INTEGER PRIMARY KEY,
        dir_path TEXT NOT NULL,
        file_name TEXT NOT NULL,
        file_size INTEGER NOT NULL,
        modified_time INTEGER NOT NULL,
        is_dir INTEGER NOT NULL,
        UNIQUE(dir_path, file_name)
    )",
    [],
)?;

// Index for fast directory lookups
conn.execute(
    "CREATE INDEX IF NOT EXISTS idx_file_index_dir ON file_index(dir_path)",
    [],
)?;
```

#### 2.3.2 Criar Modulo: `src/infrastructure/directory_index.rs`

```rust
//! Persistent directory index for fast change detection
//!
//! Stores file listings in SQLite to avoid repeated directory scans.
//! When revisiting a folder, compares stored timestamps with current
//! filesystem state to detect changes efficiently.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use rusqlite::{Connection, params};
use std::sync::Mutex;

/// Entry in the directory index
#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub is_dir: bool,
}

/// Directory scan metadata
#[derive(Debug, Clone)]
pub struct DirectoryMeta {
    pub file_count: usize,
    pub total_size: u64,
    pub last_scan: u64,
    pub scan_duration_ms: u64,
}

pub struct DirectoryIndex {
    conn: Mutex<Connection>,
}

impl DirectoryIndex {
    /// Open or create the directory index database
    pub fn open(db_path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(db_path)?;

        // Enable WAL mode for better concurrent performance
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS directory_index (
                dir_path TEXT PRIMARY KEY,
                file_count INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                last_scan_time INTEGER NOT NULL,
                scan_duration_ms INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_index (
                id INTEGER PRIMARY KEY,
                dir_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                modified_time INTEGER NOT NULL,
                is_dir INTEGER NOT NULL,
                UNIQUE(dir_path, file_name)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_file_index_dir ON file_index(dir_path)",
            [],
        )?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Get cached file list for a directory
    ///
    /// Returns None if directory is not indexed or index is stale
    pub fn get_directory(&self, dir_path: &Path) -> Option<(DirectoryMeta, Vec<IndexedFile>)> {
        let conn = self.conn.lock().ok()?;
        let dir_str = dir_path.to_string_lossy();

        // Get directory metadata
        let meta: DirectoryMeta = conn
            .query_row(
                "SELECT file_count, total_size, last_scan_time, scan_duration_ms
                 FROM directory_index WHERE dir_path = ?",
                [&dir_str],
                |row| {
                    Ok(DirectoryMeta {
                        file_count: row.get::<_, i64>(0)? as usize,
                        total_size: row.get::<_, i64>(1)? as u64,
                        last_scan: row.get::<_, i64>(2)? as u64,
                        scan_duration_ms: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .ok()?;

        // Get file entries
        let mut stmt = conn
            .prepare(
                "SELECT file_name, file_size, modified_time, is_dir
                 FROM file_index WHERE dir_path = ?",
            )
            .ok()?;

        let files: Vec<IndexedFile> = stmt
            .query_map([&dir_str], |row| {
                Ok(IndexedFile {
                    name: row.get(0)?,
                    size: row.get::<_, i64>(1)? as u64,
                    modified: row.get::<_, i64>(2)? as u64,
                    is_dir: row.get::<_, i64>(3)? != 0,
                })
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect();

        Some((meta, files))
    }

    /// Store file list for a directory
    pub fn put_directory(
        &self,
        dir_path: &Path,
        files: &[IndexedFile],
        scan_duration_ms: u64,
    ) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some("Lock poisoned".to_string()),
            )
        })?;

        let dir_str = dir_path.to_string_lossy().to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let total_size: u64 = files.iter().map(|f| f.size).sum();

        let tx = conn.transaction()?;

        // Delete old entries for this directory
        tx.execute("DELETE FROM file_index WHERE dir_path = ?", [&dir_str])?;

        // Insert new entries
        {
            let mut stmt = tx.prepare(
                "INSERT INTO file_index (dir_path, file_name, file_size, modified_time, is_dir)
                 VALUES (?, ?, ?, ?, ?)",
            )?;

            for file in files {
                stmt.execute(params![
                    &dir_str,
                    &file.name,
                    file.size as i64,
                    file.modified as i64,
                    if file.is_dir { 1 } else { 0 },
                ])?;
            }
        }

        // Update directory metadata
        tx.execute(
            "INSERT OR REPLACE INTO directory_index
             (dir_path, file_count, total_size, last_scan_time, scan_duration_ms)
             VALUES (?, ?, ?, ?, ?)",
            params![
                &dir_str,
                files.len() as i64,
                total_size as i64,
                now as i64,
                scan_duration_ms as i64,
            ],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Invalidate index for a directory (called when changes detected)
    pub fn invalidate(&self, dir_path: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock().map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some("Lock poisoned".to_string()),
            )
        })?;

        let dir_str = dir_path.to_string_lossy();

        conn.execute("DELETE FROM file_index WHERE dir_path = ?", [&dir_str])?;
        conn.execute("DELETE FROM directory_index WHERE dir_path = ?", [&dir_str])?;

        Ok(())
    }

    /// Invalidate all indexes under a parent path
    pub fn invalidate_recursive(&self, parent: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock().map_err(|_| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
                Some("Lock poisoned".to_string()),
            )
        })?;

        let parent_str = format!("{}%", parent.to_string_lossy());

        conn.execute("DELETE FROM file_index WHERE dir_path LIKE ?", [&parent_str])?;
        conn.execute("DELETE FROM directory_index WHERE dir_path LIKE ?", [&parent_str])?;

        Ok(())
    }

    /// Quick check if a directory might have changed
    ///
    /// Compares stored file count with a quick filesystem check.
    /// This is much faster than a full scan on HDDs.
    pub fn might_have_changed(&self, dir_path: &Path) -> bool {
        // Get stored metadata
        let stored = match self.get_directory(dir_path) {
            Some((meta, _)) => meta,
            None => return true, // Not indexed, needs scan
        };

        // Quick filesystem check: just count entries
        // This is a single syscall with NtQueryDirectoryFile
        use crate::infrastructure::ntfs_reader;

        if let Some(entries) = ntfs_reader::read_directory_fast(dir_path) {
            // Compare count - if different, definitely changed
            if entries.len() != stored.file_count {
                return true;
            }

            // Count matches - probably unchanged
            // (Could also compare total size for extra certainty)
            false
        } else {
            // Can't read directory, assume changed
            true
        }
    }

    /// Get statistics about the index
    pub fn stats(&self) -> Option<(usize, usize)> {
        let conn = self.conn.lock().ok()?;

        let dir_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM directory_index", [], |row| row.get(0))
            .ok()?;

        let file_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM file_index", [], |row| row.get(0))
            .ok()?;

        Some((dir_count as usize, file_count as usize))
    }
}
```

#### 2.3.3 Integrar com `load_folder()`

Adicionar verificacao de indice antes de fazer scan completo:

```rust
// Em folder_loading.rs, no inicio do thread de scan:

// OPTIMIZATION: Check persistent index first
// If directory hasn't changed, use indexed data
if !force_refresh {
    if let Some(directory_index) = &directory_index_opt {
        if !directory_index.might_have_changed(&PathBuf::from(&base_path)) {
            if let Some((meta, indexed_files)) = directory_index.get_directory(&PathBuf::from(&base_path)) {
                eprintln!("[PERF] Using persistent index ({} files, scanned {}ms ago)",
                    meta.file_count,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .saturating_sub(meta.last_scan) / 60
                );

                // Convert indexed files to FileEntry
                let entries: Vec<FileEntry> = indexed_files
                    .into_iter()
                    .filter(|f| {
                        let is_hidden = /* check from attributes if stored */;
                        !is_hidden && !f.name.starts_with('.')
                    })
                    .map(|f| FileEntry {
                        path: PathBuf::from(&base_path).join(&f.name),
                        name: f.name,
                        is_dir: f.is_dir,
                        size: if f.is_dir { 0 } else { f.size },
                        modified: f.modified,
                        folder_cover: None,
                        drive_info: None,
                        sync_status: SyncStatus::None,
                        deletion_date: None,
                        recycle_original_path: None,
                    })
                    .collect();

                // Also update memory cache
                directory_cache.put(PathBuf::from(&base_path), entries.clone());

                // Send to UI
                for chunk in entries.chunks(batch_size) {
                    if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                        return;
                    }
                    let _ = file_entry_sender.send((my_gen, chunk.to_vec()));
                    ctx.request_repaint();
                }

                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
                return;
            }
        }
    }
}

// ... rest of scan code ...

// After successful scan, store in persistent index
if let Some(directory_index) = &directory_index_opt {
    let indexed: Vec<IndexedFile> = all_entries_disk
        .iter()
        .map(|e| IndexedFile {
            name: e.name.clone(),
            size: e.size,
            modified: e.modified,
            is_dir: e.is_dir,
        })
        .collect();

    let _ = directory_index.put_directory(
        &PathBuf::from(&base_path),
        &indexed,
        scan_start.elapsed().as_millis() as u64,
    );
}
```

---

## Fase 3: USN Journal Monitoring

### 3.1 Descricao
O USN (Update Sequence Number) Journal e um log do NTFS que registra TODAS as mudancas em arquivos. E muito mais eficiente que FileSystemWatcher porque:
- Uma unica leitura captura milhares de eventos
- Nao perde eventos (FileSystemWatcher pode perder sob carga)
- Funciona mesmo quando o app estava fechado

### 3.2 Impacto
- **Beneficio:** ~90% menos overhead de monitoramento
- **Risco:** Medio (substitui sistema existente)
- **Compatibilidade:** Substitui crate `notify`

### 3.3 Analise de Conflito com `notify`

**Problema:** Manter ambos os sistemas causaria:
- Eventos duplicados
- Consumo duplo de recursos
- Complexidade desnecessaria

**Solucao:** Implementar USN Journal como substituto COMPLETO do `notify`:

```
Fase 3a: Implementar USN Journal reader (paralelo ao notify)
Fase 3b: Feature flag para alternar entre USN e notify
Fase 3c: Deprecar notify apos validacao
Fase 3d: Remover notify completamente
```

### 3.4 Implementacao Detalhada

#### 3.4.1 Criar Modulo: `src/infrastructure/usn_journal.rs`

```rust
//! USN Journal reader for efficient filesystem change detection
//!
//! The USN (Update Sequence Number) Journal is an NTFS feature that logs
//! all file changes. Reading it is much more efficient than using
//! FileSystemWatcher because:
//!
//! 1. A single read can capture thousands of changes
//! 2. It never misses events (unlike FileSystemWatcher under load)
//! 3. It works even for changes that happened while the app was closed
//!
//! # Requirements
//! - NTFS filesystem (doesn't work on FAT32, exFAT, or network drives)
//! - Read access to the volume (usually requires running as user, not admin)
//!
//! # Usage
//! ```
//! let journal = UsnJournal::open('C')?;
//! let changes = journal.read_changes(last_usn)?;
//! for change in changes {
//!     println!("{}: {:?}", change.file_name, change.reason);
//! }
//! ```

use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    FILE_FLAG_BACKUP_SEMANTICS,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{
    FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL,
};

/// USN Journal change reasons (bitmask)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsnReason(pub u32);

impl UsnReason {
    pub const DATA_OVERWRITE: u32 = 0x00000001;
    pub const DATA_EXTEND: u32 = 0x00000002;
    pub const DATA_TRUNCATION: u32 = 0x00000004;
    pub const NAMED_DATA_OVERWRITE: u32 = 0x00000010;
    pub const NAMED_DATA_EXTEND: u32 = 0x00000020;
    pub const NAMED_DATA_TRUNCATION: u32 = 0x00000040;
    pub const FILE_CREATE: u32 = 0x00000100;
    pub const FILE_DELETE: u32 = 0x00000200;
    pub const EA_CHANGE: u32 = 0x00000400;
    pub const SECURITY_CHANGE: u32 = 0x00000800;
    pub const RENAME_OLD_NAME: u32 = 0x00001000;
    pub const RENAME_NEW_NAME: u32 = 0x00002000;
    pub const INDEXABLE_CHANGE: u32 = 0x00004000;
    pub const BASIC_INFO_CHANGE: u32 = 0x00008000;
    pub const HARD_LINK_CHANGE: u32 = 0x00010000;
    pub const COMPRESSION_CHANGE: u32 = 0x00020000;
    pub const ENCRYPTION_CHANGE: u32 = 0x00040000;
    pub const OBJECT_ID_CHANGE: u32 = 0x00080000;
    pub const REPARSE_POINT_CHANGE: u32 = 0x00100000;
    pub const STREAM_CHANGE: u32 = 0x00200000;
    pub const CLOSE: u32 = 0x80000000;

    pub fn is_create(&self) -> bool {
        (self.0 & Self::FILE_CREATE) != 0
    }

    pub fn is_delete(&self) -> bool {
        (self.0 & Self::FILE_DELETE) != 0
    }

    pub fn is_rename(&self) -> bool {
        (self.0 & (Self::RENAME_OLD_NAME | Self::RENAME_NEW_NAME)) != 0
    }

    pub fn is_modify(&self) -> bool {
        (self.0 & (Self::DATA_OVERWRITE | Self::DATA_EXTEND | Self::DATA_TRUNCATION)) != 0
    }

    pub fn is_close(&self) -> bool {
        (self.0 & Self::CLOSE) != 0
    }
}

/// A change record from the USN Journal
#[derive(Debug, Clone)]
pub struct UsnRecord {
    /// The USN of this record
    pub usn: i64,
    /// File reference number (unique ID on the volume)
    pub file_reference_number: u64,
    /// Parent directory reference number
    pub parent_reference_number: u64,
    /// Reason flags for the change
    pub reason: UsnReason,
    /// File name (not full path - just the name)
    pub file_name: String,
    /// File attributes
    pub file_attributes: u32,
}

impl UsnRecord {
    pub fn is_directory(&self) -> bool {
        (self.file_attributes & 0x10) != 0 // FILE_ATTRIBUTE_DIRECTORY
    }
}

/// USN Journal reader for a volume
pub struct UsnJournal {
    handle: HANDLE,
    journal_id: u64,
    first_usn: i64,
    next_usn: i64,
    drive_letter: char,
}

#[repr(C)]
struct UsnJournalData {
    usn_journal_id: u64,
    first_usn: i64,
    next_usn: i64,
    lowest_valid_usn: i64,
    max_usn: i64,
    maximum_size: u64,
    allocation_delta: u64,
}

#[repr(C)]
struct ReadUsnJournalData {
    start_usn: i64,
    reason_mask: u32,
    return_only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    usn_journal_id: u64,
}

#[repr(C)]
struct UsnRecordV2 {
    record_length: u32,
    major_version: u16,
    minor_version: u16,
    file_reference_number: u64,
    parent_file_reference_number: u64,
    usn: i64,
    time_stamp: i64,
    reason: u32,
    source_info: u32,
    security_id: u32,
    file_attributes: u32,
    file_name_length: u16,
    file_name_offset: u16,
    // file_name follows (variable length UTF-16)
}

impl UsnJournal {
    /// Open the USN Journal for a volume
    ///
    /// # Arguments
    /// * `drive_letter` - The drive letter (e.g., 'C')
    ///
    /// # Returns
    /// * `Ok(UsnJournal)` - Successfully opened journal
    /// * `Err(String)` - Failed to open (not NTFS, permissions, etc.)
    pub fn open(drive_letter: char) -> Result<Self, String> {
        let volume_path = format!("\\\\.\\{}:", drive_letter.to_ascii_uppercase());
        let wide_path: Vec<u16> = volume_path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_path.as_ptr()),
                0x80000000, // GENERIC_READ
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        };

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return Err(format!("Failed to open volume {}", drive_letter)),
        };

        // Query journal info
        let mut journal_data = UsnJournalData {
            usn_journal_id: 0,
            first_usn: 0,
            next_usn: 0,
            lowest_valid_usn: 0,
            max_usn: 0,
            maximum_size: 0,
            allocation_delta: 0,
        };
        let mut bytes_returned: u32 = 0;

        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_QUERY_USN_JOURNAL,
                None,
                0,
                Some(&mut journal_data as *mut _ as *mut std::ffi::c_void),
                std::mem::size_of::<UsnJournalData>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if result.is_err() {
            unsafe { let _ = CloseHandle(handle); }
            return Err(format!("USN Journal not available on drive {}", drive_letter));
        }

        Ok(Self {
            handle,
            journal_id: journal_data.usn_journal_id,
            first_usn: journal_data.first_usn,
            next_usn: journal_data.next_usn,
            drive_letter: drive_letter.to_ascii_uppercase(),
        })
    }

    /// Get the current USN (use as starting point for future reads)
    pub fn current_usn(&self) -> i64 {
        self.next_usn
    }

    /// Get the first valid USN in the journal
    pub fn first_usn(&self) -> i64 {
        self.first_usn
    }

    /// Read all changes since a given USN
    ///
    /// # Arguments
    /// * `start_usn` - Read changes after this USN (use 0 for all, or previous current_usn())
    ///
    /// # Returns
    /// * Vector of change records
    /// * New USN to use for next read
    pub fn read_changes(&self, start_usn: i64) -> Result<(Vec<UsnRecord>, i64), String> {
        let mut records = Vec::new();
        let mut current_usn = start_usn;

        // 64KB buffer for reading records
        let mut buffer = vec![0u8; 65536];

        loop {
            let read_data = ReadUsnJournalData {
                start_usn: current_usn,
                reason_mask: 0xFFFFFFFF, // All reasons
                return_only_on_close: 0,
                timeout: 0,
                bytes_to_wait_for: 0,
                usn_journal_id: self.journal_id,
            };

            let mut bytes_returned: u32 = 0;

            let result = unsafe {
                DeviceIoControl(
                    self.handle,
                    FSCTL_READ_USN_JOURNAL,
                    Some(&read_data as *const _ as *const std::ffi::c_void),
                    std::mem::size_of::<ReadUsnJournalData>() as u32,
                    Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
                    buffer.len() as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            };

            if result.is_err() || bytes_returned < 8 {
                break;
            }

            // First 8 bytes is the next USN
            let next_usn = i64::from_le_bytes(buffer[0..8].try_into().unwrap());

            if next_usn == current_usn {
                // No more records
                break;
            }

            // Parse records after the 8-byte header
            let mut offset = 8usize;
            while offset < bytes_returned as usize {
                let record_ptr = unsafe { buffer.as_ptr().add(offset) as *const UsnRecordV2 };
                let record = unsafe { &*record_ptr };

                if record.record_length == 0 {
                    break;
                }

                // Extract file name (UTF-16)
                let name_start = offset + record.file_name_offset as usize;
                let name_len = (record.file_name_length / 2) as usize;

                if name_start + name_len * 2 <= bytes_returned as usize {
                    let name_ptr = unsafe { buffer.as_ptr().add(name_start) as *const u16 };
                    let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
                    let file_name = OsString::from_wide(name_slice).to_string_lossy().into_owned();

                    records.push(UsnRecord {
                        usn: record.usn,
                        file_reference_number: record.file_reference_number,
                        parent_reference_number: record.parent_file_reference_number,
                        reason: UsnReason(record.reason),
                        file_name,
                        file_attributes: record.file_attributes,
                    });
                }

                offset += record.record_length as usize;
            }

            current_usn = next_usn;
        }

        Ok((records, current_usn))
    }

    /// Filter changes to only those in specific directories
    ///
    /// Since USN records only have file reference numbers (not paths),
    /// we need to filter by parent reference numbers. This requires
    /// first resolving the monitored paths to their reference numbers.
    pub fn filter_by_directories(
        &self,
        records: Vec<UsnRecord>,
        monitored_dirs: &HashSet<u64>, // Parent reference numbers
    ) -> Vec<UsnRecord> {
        records
            .into_iter()
            .filter(|r| monitored_dirs.contains(&r.parent_reference_number))
            .collect()
    }
}

impl Drop for UsnJournal {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Get the file reference number for a path
///
/// This is needed to filter USN records by directory
pub fn get_file_reference_number(path: &std::path::Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetFileInformationByHandle;
    use windows::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION;

    let wide_path: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide_path.as_ptr()),
            0, // No access needed
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS, // Required for directories
            None,
        )
    };

    let handle = match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => h,
        _ => return None,
    };

    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let result = unsafe { GetFileInformationByHandle(handle, &mut info) };
    unsafe { let _ = CloseHandle(handle); }

    if result.is_ok() {
        // Combine high and low parts into 64-bit reference number
        let file_ref = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
        Some(file_ref)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_journal() {
        // Should work on C: drive (most systems have NTFS)
        let result = UsnJournal::open('C');
        assert!(result.is_ok(), "Failed to open USN Journal on C:");

        let journal = result.unwrap();
        assert!(journal.current_usn() > 0);
        println!("Current USN: {}", journal.current_usn());
    }

    #[test]
    fn test_read_recent_changes() {
        let journal = match UsnJournal::open('C') {
            Ok(j) => j,
            Err(_) => return, // Skip if journal not available
        };

        // Read last 1000 records or so
        let start = journal.current_usn() - 1000000; // ~1MB of records
        let (records, _) = journal.read_changes(start.max(journal.first_usn())).unwrap();

        println!("Read {} records", records.len());

        // Should have some records
        assert!(!records.is_empty() || journal.current_usn() == journal.first_usn());
    }
}
```

#### 3.4.2 Criar Worker: `src/workers/usn_watcher.rs`

```rust
//! USN Journal watcher worker
//!
//! Polls the USN Journal periodically to detect filesystem changes.
//! More efficient than FileSystemWatcher, especially for multiple directories.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::infrastructure::usn_journal::{UsnJournal, UsnRecord, get_file_reference_number};

/// Events sent to the main thread
#[derive(Debug, Clone)]
pub enum FsEvent {
    /// File created
    Created(PathBuf),
    /// File deleted
    Deleted(PathBuf),
    /// File modified
    Modified(PathBuf),
    /// File renamed (old_path, new_path)
    Renamed(PathBuf, PathBuf),
}

/// Shared state for the USN watcher
pub struct UsnWatcherState {
    /// Directories being monitored (path -> file reference number)
    monitored_dirs: Mutex<HashSet<(PathBuf, u64)>>,
    /// Last USN we processed per drive
    last_usn: Mutex<std::collections::HashMap<char, i64>>,
}

impl UsnWatcherState {
    pub fn new() -> Self {
        Self {
            monitored_dirs: Mutex::new(HashSet::new()),
            last_usn: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Add a directory to monitor
    pub fn watch(&self, path: PathBuf) {
        if let Some(file_ref) = get_file_reference_number(&path) {
            if let Ok(mut dirs) = self.monitored_dirs.lock() {
                dirs.insert((path, file_ref));
            }
        }
    }

    /// Remove a directory from monitoring
    pub fn unwatch(&self, path: &PathBuf) {
        if let Ok(mut dirs) = self.monitored_dirs.lock() {
            dirs.retain(|(p, _)| p != path);
        }
    }

    /// Get all monitored file reference numbers for a drive
    fn get_monitored_refs(&self, drive: char) -> HashSet<u64> {
        if let Ok(dirs) = self.monitored_dirs.lock() {
            dirs.iter()
                .filter(|(p, _)| {
                    p.to_string_lossy()
                        .chars()
                        .next()
                        .map(|c| c.to_ascii_uppercase() == drive)
                        .unwrap_or(false)
                })
                .map(|(_, ref_num)| *ref_num)
                .collect()
        } else {
            HashSet::new()
        }
    }

    /// Get path for a file reference number
    fn get_path_for_ref(&self, file_ref: u64) -> Option<PathBuf> {
        if let Ok(dirs) = self.monitored_dirs.lock() {
            dirs.iter()
                .find(|(_, r)| *r == file_ref)
                .map(|(p, _)| p.clone())
        } else {
            None
        }
    }
}

/// Spawn the USN watcher worker thread
pub fn spawn_usn_watcher(
    state: Arc<UsnWatcherState>,
    event_sender: Sender<FsEvent>,
    poll_interval: Duration,
) {
    std::thread::spawn(move || {
        // Open journals for each drive we encounter
        let mut journals: std::collections::HashMap<char, UsnJournal> =
            std::collections::HashMap::new();

        loop {
            // Get unique drives from monitored directories
            let drives: HashSet<char> = if let Ok(dirs) = state.monitored_dirs.lock() {
                dirs.iter()
                    .filter_map(|(p, _)| {
                        p.to_string_lossy()
                            .chars()
                            .next()
                            .map(|c| c.to_ascii_uppercase())
                    })
                    .collect()
            } else {
                HashSet::new()
            };

            // Process each drive
            for drive in drives {
                // Open journal if not already open
                if !journals.contains_key(&drive) {
                    if let Ok(journal) = UsnJournal::open(drive) {
                        // Initialize last USN
                        if let Ok(mut last_usn) = state.last_usn.lock() {
                            last_usn.entry(drive).or_insert(journal.current_usn());
                        }
                        journals.insert(drive, journal);
                    }
                }

                // Read changes
                if let Some(journal) = journals.get(&drive) {
                    let start_usn = state.last_usn
                        .lock()
                        .ok()
                        .and_then(|m| m.get(&drive).copied())
                        .unwrap_or(journal.current_usn());

                    if let Ok((records, new_usn)) = journal.read_changes(start_usn) {
                        // Update last USN
                        if let Ok(mut last_usn) = state.last_usn.lock() {
                            last_usn.insert(drive, new_usn);
                        }

                        // Filter to monitored directories
                        let monitored_refs = state.get_monitored_refs(drive);
                        let relevant: Vec<UsnRecord> = records
                            .into_iter()
                            .filter(|r| monitored_refs.contains(&r.parent_reference_number))
                            .collect();

                        // Convert to events
                        for record in relevant {
                            // Get parent directory path
                            if let Some(parent_path) = state.get_path_for_ref(record.parent_reference_number) {
                                let file_path = parent_path.join(&record.file_name);

                                // Only send events for completed operations (CLOSE flag)
                                if record.reason.is_close() {
                                    if record.reason.is_create() {
                                        let _ = event_sender.send(FsEvent::Created(file_path));
                                    } else if record.reason.is_delete() {
                                        let _ = event_sender.send(FsEvent::Deleted(file_path));
                                    } else if record.reason.is_modify() {
                                        let _ = event_sender.send(FsEvent::Modified(file_path));
                                    }
                                    // Rename events would need to match old/new pairs - more complex
                                }
                            }
                        }
                    }
                }
            }

            std::thread::sleep(poll_interval);
        }
    });
}
```

#### 3.4.3 Plano de Transicao do `notify`

**Passo 1:** Adicionar feature flag em `Cargo.toml`:

```toml
[features]
default = ["notify-watcher"]
notify-watcher = ["notify"]
usn-watcher = []
```

**Passo 2:** Condicionar codigo do notify:

```rust
// Em app/operations/watcher.rs
#[cfg(feature = "notify-watcher")]
pub fn watch_current_folder(&mut self) {
    // ... implementacao atual com notify ...
}

#[cfg(feature = "usn-watcher")]
pub fn watch_current_folder(&mut self) {
    // ... implementacao com USN Journal ...
}
```

**Passo 3:** Testar extensivamente com `--features usn-watcher`

**Passo 4:** Mudar default para `usn-watcher` apos validacao

**Passo 5:** Remover `notify` da lista de dependencias

---

## Fase 4: I/O Priority por Handle

### 4.1 Descricao
Alem da prioridade por thread (ja implementada), definir prioridade de I/O por handle de arquivo usando `SetFileInformationByHandle`. Isso permite controle mais fino sobre quais operacoes de I/O devem ter prioridade.

### 4.2 Analise de Compatibilidade

**Estado atual:** Thread priority via `THREAD_MODE_BACKGROUND_BEGIN` (linha 189 de io_priority.rs)

**Potencial conflito:** `THREAD_MODE_BACKGROUND_BEGIN` ja define prioridade de I/O para a thread toda. Adicionar prioridade por handle pode:
- Ser redundante (ambos fazem a mesma coisa)
- Causar comportamento inesperado se prioridades conflitarem

**Decisao:** Usar I/O Priority por handle APENAS para operacoes especificas que precisam de prioridade diferente da thread. Por exemplo:
- Thread em background, mas uma operacao especifica precisa de prioridade normal
- Thread normal, mas prefetch deve ser low priority

### 4.3 Implementacao

Adicionar em `src/infrastructure/windows/file_flags.rs`:

```rust
use windows::Win32::Storage::FileSystem::{
    SetFileInformationByHandle, FileIoPriorityHintInfo,
    FILE_IO_PRIORITY_HINT_INFO,
};
use windows::Win32::Foundation::HANDLE;
use std::os::windows::io::AsRawHandle;

/// I/O priority levels for file handles
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIoPriority {
    /// Very low priority - background operations
    VeryLow = 0,
    /// Low priority - prefetch, non-urgent reads
    Low = 1,
    /// Normal priority - user-initiated operations
    Normal = 2,
    // Note: High/Critical are reserved for system use
}

/// Set I/O priority for a file handle
///
/// This complements thread-level priority by allowing fine-grained control
/// over specific file operations.
///
/// # Arguments
/// * `file` - The file to set priority on
/// * `priority` - The desired I/O priority
///
/// # Example
/// ```
/// let file = open_sequential(path)?;
/// set_file_io_priority(&file, FileIoPriority::Low)?; // Prefetch, low priority
/// ```
pub fn set_file_io_priority<F: AsRawHandle>(
    file: &F,
    priority: FileIoPriority,
) -> std::io::Result<()> {
    let handle = HANDLE(file.as_raw_handle() as isize);

    // IoPriorityHintVeryLow = 0, IoPriorityHintLow = 1, IoPriorityHintNormal = 2
    let hint = FILE_IO_PRIORITY_HINT_INFO {
        PriorityHint: priority as i32,
    };

    let result = unsafe {
        SetFileInformationByHandle(
            handle,
            FileIoPriorityHintInfo,
            &hint as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<FILE_IO_PRIORITY_HINT_INFO>() as u32,
        )
    };

    if result.is_ok() {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Open a file with sequential scan AND low I/O priority
///
/// Ideal for prefetch operations that shouldn't interfere with user actions.
pub fn open_sequential_low_priority(path: &Path) -> std::io::Result<File> {
    let file = open_sequential(path)?;
    let _ = set_file_io_priority(&file, FileIoPriority::Low);
    Ok(file)
}

/// Open a file with sequential scan AND very low I/O priority
///
/// Ideal for background operations like folder cover scanning.
pub fn open_sequential_background(path: &Path) -> std::io::Result<File> {
    let file = open_sequential(path)?;
    let _ = set_file_io_priority(&file, FileIoPriority::VeryLow);
    Ok(file)
}
```

### 4.4 Uso em Thumbnail Worker

```rust
// Em thumbnail_worker.rs, modificar para usar prioridade apropriada:

fn try_image_crate_extraction(path: &Path, priority: IOPriority) -> Option<(Vec<u8>, u32, u32)> {
    use crate::infrastructure::windows::file_flags::{
        open_sequential, open_sequential_low_priority, open_sequential_background
    };

    // Choose file opener based on priority
    let file = match priority {
        IOPriority::Interactive => open_sequential(path).ok()?,
        IOPriority::Prefetch => open_sequential_low_priority(path).ok()?,
        IOPriority::Background => open_sequential_background(path).ok()?,
    };

    // ... rest of extraction code ...
}
```

---

## Fase 5: MFT Direct Read (OPCIONAL - Avancado)

> **⛔ NAO IMPLEMENTAR SEM ORDEM EXPLICITA DO USUARIO**
>
> Esta fase so deve ser iniciada quando o usuario solicitar explicitamente.
> Nao faz parte do fluxo padrao de otimizacoes.

### 5.1 Descricao

A Master File Table (MFT) e a estrutura central do NTFS que contem metadados de TODOS os arquivos do volume. Ler diretamente da MFT permite enumerar milhoes de arquivos em segundos - e o que o **Everything (voidtools)** usa.

### 5.2 Quando Usar

**Casos de uso adequados:**
- Busca global em todo o disco
- Indexacao de arquivos para busca rapida
- Estatisticas de uso de disco
- Ferramenta tipo "disk analyzer"

**NAO adequado para:**
- Navegacao pasta a pasta (NtQueryDirectoryFile ja e excelente)
- Usuarios sem privilegios de admin

### 5.3 Requisitos

| Requisito | Motivo |
|-----------|--------|
| Privilegios de Administrador | Acesso direto ao volume |
| Filesystem NTFS | MFT e especifica do NTFS |
| Windows Vista+ | APIs necessarias |

### 5.4 Impacto

- **Beneficio:** ~99% mais rapido para enumerar volume inteiro
- **Risco:** Alto (estruturas internas, requer admin)
- **Complexidade:** Muito alta

### 5.5 Implementacao Detalhada

#### 5.5.1 Criar Modulo: `src/infrastructure/mft_reader.rs`

```rust
//! Direct MFT (Master File Table) reader for ultra-fast file enumeration
//!
//! This module reads the NTFS Master File Table directly, bypassing the
//! filesystem layer entirely. This is the fastest possible way to enumerate
//! files on an NTFS volume.
//!
//! # Requirements
//! - Administrator privileges
//! - NTFS filesystem
//! - Windows Vista or later
//!
//! # Performance
//! Can enumerate millions of files in seconds - same technique used by
//! "Everything" search tool from voidtools.
//!
//! # Warning
//! This reads internal NTFS structures. While these are stable, they could
//! theoretically change in future Windows versions.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_VOLUME_DATA;

/// MFT file record (simplified)
#[derive(Debug, Clone)]
pub struct MftEntry {
    /// File reference number (index in MFT)
    pub file_ref: u64,
    /// Parent directory reference
    pub parent_ref: u64,
    /// File name
    pub name: String,
    /// File size in bytes
    pub size: u64,
    /// Is this a directory?
    pub is_dir: bool,
    /// File attributes
    pub attributes: u32,
    /// Creation time (Windows FILETIME)
    pub created: u64,
    /// Modification time (Windows FILETIME)
    pub modified: u64,
}

/// NTFS volume data from FSCTL_GET_NTFS_VOLUME_DATA
#[repr(C)]
#[derive(Default)]
struct NtfsVolumeData {
    volume_serial_number: u64,
    number_sectors: u64,
    total_clusters: u64,
    free_clusters: u64,
    total_reserved: u64,
    bytes_per_sector: u32,
    bytes_per_cluster: u32,
    bytes_per_file_record_segment: u32,
    clusters_per_file_record_segment: u32,
    mft_valid_data_length: u64,
    mft_start_lcn: u64,
    mft2_start_lcn: u64,
    mft_zone_start: u64,
    mft_zone_end: u64,
}

/// MFT attribute types we care about
const ATTRIBUTE_STANDARD_INFORMATION: u32 = 0x10;
const ATTRIBUTE_FILE_NAME: u32 = 0x30;
const ATTRIBUTE_DATA: u32 = 0x80;

/// File record flags
const FILE_RECORD_IN_USE: u16 = 0x0001;
const FILE_RECORD_IS_DIRECTORY: u16 = 0x0002;

/// Standard Information attribute (timestamps)
#[repr(C, packed)]
struct StandardInformation {
    creation_time: u64,
    modification_time: u64,
    mft_modification_time: u64,
    access_time: u64,
    file_attributes: u32,
    // ... more fields we don't need
}

/// File Name attribute
#[repr(C, packed)]
struct FileNameAttribute {
    parent_directory: u64,
    creation_time: u64,
    modification_time: u64,
    mft_modification_time: u64,
    access_time: u64,
    allocated_size: u64,
    data_size: u64,
    file_attributes: u32,
    reparse_value: u32,
    name_length: u8,
    name_type: u8,
    // name follows (UTF-16)
}

/// MFT reader for a volume
pub struct MftReader {
    handle: HANDLE,
    volume_data: NtfsVolumeData,
    drive_letter: char,
}

impl MftReader {
    /// Open MFT reader for a volume
    ///
    /// # Arguments
    /// * `drive_letter` - The drive letter (e.g., 'C')
    ///
    /// # Returns
    /// * `Ok(MftReader)` - Successfully opened
    /// * `Err(String)` - Failed (not admin, not NTFS, etc.)
    ///
    /// # Requires
    /// Administrator privileges
    pub fn open(drive_letter: char) -> Result<Self, String> {
        let volume_path = format!("\\\\.\\{}:", drive_letter.to_ascii_uppercase());
        let wide_path: Vec<u16> = volume_path.encode_utf16().chain(std::iter::once(0)).collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(wide_path.as_ptr()),
                FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        };

        let handle = match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => h,
            _ => return Err(format!(
                "Failed to open volume {}. Are you running as Administrator?",
                drive_letter
            )),
        };

        // Get NTFS volume data
        let mut volume_data = NtfsVolumeData::default();
        let mut bytes_returned: u32 = 0;

        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                None,
                0,
                Some(&mut volume_data as *mut _ as *mut std::ffi::c_void),
                std::mem::size_of::<NtfsVolumeData>() as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if result.is_err() {
            unsafe { let _ = CloseHandle(handle); }
            return Err(format!(
                "Failed to get NTFS volume data for {}. Is it NTFS?",
                drive_letter
            ));
        }

        Ok(Self {
            handle,
            volume_data,
            drive_letter: drive_letter.to_ascii_uppercase(),
        })
    }

    /// Read all file entries from the MFT
    ///
    /// # Returns
    /// HashMap of file_ref -> MftEntry for all files on the volume
    ///
    /// # Performance
    /// Typically completes in 1-5 seconds even for millions of files
    pub fn read_all_entries(&self) -> Result<HashMap<u64, MftEntry>, String> {
        use windows::Win32::System::Ioctl::FSCTL_GET_NTFS_FILE_RECORD;

        let record_size = self.volume_data.bytes_per_file_record_segment as usize;
        let mft_size = self.volume_data.mft_valid_data_length;
        let total_records = mft_size as usize / record_size;

        let mut entries = HashMap::with_capacity(total_records);
        let mut buffer = vec![0u8; record_size + 8]; // +8 for NTFS_FILE_RECORD_OUTPUT_BUFFER header

        // Input structure for FSCTL_GET_NTFS_FILE_RECORD
        #[repr(C)]
        struct NtfsFileRecordInput {
            file_reference_number: u64,
        }

        eprintln!("[MFT] Reading {} records ({} MB MFT)",
            total_records,
            mft_size / 1024 / 1024
        );

        let start = std::time::Instant::now();

        for file_ref in 0..total_records as u64 {
            let input = NtfsFileRecordInput {
                file_reference_number: file_ref,
            };

            let mut bytes_returned: u32 = 0;

            let result = unsafe {
                DeviceIoControl(
                    self.handle,
                    FSCTL_GET_NTFS_FILE_RECORD,
                    Some(&input as *const _ as *const std::ffi::c_void),
                    std::mem::size_of::<NtfsFileRecordInput>() as u32,
                    Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
                    buffer.len() as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            };

            if result.is_err() || bytes_returned < 8 {
                continue;
            }

            // Parse the file record
            // Skip 8-byte header (NTFS_FILE_RECORD_OUTPUT_BUFFER)
            if let Some(entry) = self.parse_file_record(&buffer[8..], file_ref) {
                entries.insert(file_ref, entry);
            }

            // Progress logging every 100k records
            if file_ref % 100_000 == 0 && file_ref > 0 {
                eprintln!("[MFT] Processed {} records...", file_ref);
            }
        }

        eprintln!("[MFT] Read {} entries in {:.2}s",
            entries.len(),
            start.elapsed().as_secs_f64()
        );

        Ok(entries)
    }

    /// Parse a single MFT file record
    fn parse_file_record(&self, data: &[u8], file_ref: u64) -> Option<MftEntry> {
        if data.len() < 48 {
            return None;
        }

        // Check FILE signature
        if &data[0..4] != b"FILE" {
            return None;
        }

        // Get flags
        let flags = u16::from_le_bytes([data[22], data[23]]);

        // Skip if not in use
        if (flags & FILE_RECORD_IN_USE) == 0 {
            return None;
        }

        let is_dir = (flags & FILE_RECORD_IS_DIRECTORY) != 0;

        // Get first attribute offset
        let attr_offset = u16::from_le_bytes([data[20], data[21]]) as usize;

        let mut name = String::new();
        let mut parent_ref = 0u64;
        let mut size = 0u64;
        let mut attributes = 0u32;
        let mut created = 0u64;
        let mut modified = 0u64;

        // Walk through attributes
        let mut offset = attr_offset;
        while offset + 4 < data.len() {
            let attr_type = u32::from_le_bytes([
                data[offset], data[offset + 1],
                data[offset + 2], data[offset + 3]
            ]);

            if attr_type == 0xFFFFFFFF {
                break; // End marker
            }

            let attr_len = u32::from_le_bytes([
                data[offset + 4], data[offset + 5],
                data[offset + 6], data[offset + 7]
            ]) as usize;

            if attr_len == 0 || offset + attr_len > data.len() {
                break;
            }

            match attr_type {
                ATTRIBUTE_STANDARD_INFORMATION => {
                    // Non-resident flag at offset 8
                    if data[offset + 8] == 0 {
                        // Resident - content at offset 24
                        let content_offset = u16::from_le_bytes([
                            data[offset + 20], data[offset + 21]
                        ]) as usize;

                        if offset + content_offset + 32 <= data.len() {
                            let si = &data[offset + content_offset..];
                            created = u64::from_le_bytes(si[0..8].try_into().ok()?);
                            modified = u64::from_le_bytes(si[8..16].try_into().ok()?);
                            attributes = u32::from_le_bytes(si[32..36].try_into().ok()?);
                        }
                    }
                }
                ATTRIBUTE_FILE_NAME => {
                    if data[offset + 8] == 0 {
                        let content_offset = u16::from_le_bytes([
                            data[offset + 20], data[offset + 21]
                        ]) as usize;

                        if offset + content_offset + 66 <= data.len() {
                            let fn_data = &data[offset + content_offset..];

                            parent_ref = u64::from_le_bytes(fn_data[0..8].try_into().ok()?) & 0x0000FFFFFFFFFFFF;
                            let name_len = fn_data[64] as usize;
                            let name_type = fn_data[65];

                            // Prefer Win32 name (type 1 or 3) over DOS name (type 2)
                            if name_type != 2 && offset + content_offset + 66 + name_len * 2 <= data.len() {
                                let name_bytes = &fn_data[66..66 + name_len * 2];
                                let wide: Vec<u16> = name_bytes
                                    .chunks_exact(2)
                                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                                    .collect();
                                name = OsString::from_wide(&wide).to_string_lossy().into_owned();
                            }
                        }
                    }
                }
                ATTRIBUTE_DATA => {
                    if data[offset + 8] == 0 {
                        // Resident data - get size
                        size = u32::from_le_bytes([
                            data[offset + 16], data[offset + 17],
                            data[offset + 18], data[offset + 19]
                        ]) as u64;
                    } else {
                        // Non-resident data - real size at offset 48
                        if offset + 56 <= data.len() {
                            size = u64::from_le_bytes(
                                data[offset + 48..offset + 56].try_into().ok()?
                            );
                        }
                    }
                }
                _ => {}
            }

            offset += attr_len;
        }

        // Skip entries without names (system metafiles like $MFT)
        if name.is_empty() || name.starts_with('$') {
            return None;
        }

        Some(MftEntry {
            file_ref,
            parent_ref,
            name,
            size,
            is_dir,
            attributes,
            created,
            modified,
        })
    }

    /// Build full paths from MFT entries
    ///
    /// MFT entries only contain parent references, not full paths.
    /// This function resolves all entries to their full paths.
    pub fn build_paths(&self, entries: &HashMap<u64, MftEntry>) -> HashMap<u64, PathBuf> {
        let mut paths: HashMap<u64, PathBuf> = HashMap::with_capacity(entries.len());

        // Root directory is always ref 5
        paths.insert(5, PathBuf::from(format!("{}:\\", self.drive_letter)));

        // Iteratively resolve paths (may need multiple passes for deep nesting)
        let mut resolved = 1;
        while resolved > 0 {
            resolved = 0;
            for (file_ref, entry) in entries {
                if paths.contains_key(file_ref) {
                    continue;
                }

                if let Some(parent_path) = paths.get(&entry.parent_ref) {
                    let full_path = parent_path.join(&entry.name);
                    paths.insert(*file_ref, full_path);
                    resolved += 1;
                }
            }
        }

        paths
    }

    /// Search for files matching a pattern across the entire volume
    ///
    /// This is MUCH faster than recursive directory walking
    pub fn search(&self, pattern: &str) -> Result<Vec<(PathBuf, MftEntry)>, String> {
        let entries = self.read_all_entries()?;
        let paths = self.build_paths(&entries);

        let pattern_lower = pattern.to_lowercase();

        let results: Vec<_> = entries
            .iter()
            .filter(|(_, e)| e.name.to_lowercase().contains(&pattern_lower))
            .filter_map(|(ref_num, entry)| {
                paths.get(ref_num).map(|path| (path.clone(), entry.clone()))
            })
            .collect();

        Ok(results)
    }
}

impl Drop for MftReader {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Check if we have admin privileges (required for MFT access)
pub fn has_admin_privileges() -> bool {
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;

        let result = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );

        let _ = CloseHandle(token);

        result.is_ok() && elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_check() {
        let is_admin = has_admin_privileges();
        println!("Running as admin: {}", is_admin);
        // Don't assert - test should pass either way
    }

    #[test]
    fn test_mft_reader() {
        if !has_admin_privileges() {
            eprintln!("Skipping MFT test - not running as admin");
            return;
        }

        let reader = MftReader::open('C');
        assert!(reader.is_ok(), "Failed to open MFT reader");

        let reader = reader.unwrap();
        let entries = reader.read_all_entries();
        assert!(entries.is_ok(), "Failed to read MFT entries");

        let entries = entries.unwrap();
        println!("Found {} MFT entries", entries.len());
        assert!(!entries.is_empty());
    }
}
```

### 5.6 Integracao com o Aplicativo

#### 5.6.1 Busca Global (Feature Futura)

```rust
// Em um futuro modulo de busca global:
pub fn global_search(query: &str) -> Vec<SearchResult> {
    // Verifica se tem privilegios de admin
    if !mft_reader::has_admin_privileges() {
        // Fallback para busca recursiva tradicional
        return traditional_search(query);
    }

    // Usa MFT para busca ultra-rapida
    let mut results = Vec::new();

    for drive in get_ntfs_drives() {
        if let Ok(reader) = MftReader::open(drive) {
            if let Ok(matches) = reader.search(query) {
                results.extend(matches.into_iter().map(|(path, entry)| {
                    SearchResult {
                        path,
                        name: entry.name,
                        size: entry.size,
                        is_dir: entry.is_dir,
                        modified: filetime_to_unix(entry.modified),
                    }
                }));
            }
        }
    }

    results
}
```

### 5.7 Consideracoes de Seguranca

| Aspecto | Consideracao |
|---------|--------------|
| Privilegios | Requer execucao como Administrador |
| Estruturas internas | Podem mudar em futuras versoes do Windows |
| NTFS only | Nao funciona em FAT32, exFAT, ReFS |
| Risco | Leitura apenas, nao modifica nada |

### 5.8 Quando NAO Usar

- Navegacao pasta a pasta (usar NtQueryDirectoryFile)
- Usuarios sem privilegios de admin
- Drives non-NTFS
- Sistemas onde estabilidade e mais importante que velocidade

---

## Checklist de Implementacao

### Fase 1: FILE_FLAG_SEQUENTIAL_SCAN
- [ ] Criar `src/infrastructure/windows/file_flags.rs`
- [ ] Registrar em `src/infrastructure/windows/mod.rs`
- [ ] Modificar `try_image_crate_extraction()` em thumbnail_worker.rs
- [ ] Testar com imagens grandes em HDD
- [ ] Medir diferenca de performance (antes/depois)

### Fase 2: Persistent Thumbnail Index
- [ ] Adicionar tabelas em `disk_cache.rs` init_db()
- [ ] Criar `src/infrastructure/directory_index.rs`
- [ ] Registrar em `src/infrastructure/mod.rs`
- [ ] Adicionar campo `directory_index` em ImageViewerApp
- [ ] Integrar verificacao em `load_folder()`
- [ ] Integrar invalidacao com cache de diretorios
- [ ] Testar navegacao repetida em mesmas pastas

### Fase 3: USN Journal
- [ ] Criar `src/infrastructure/usn_journal.rs`
- [ ] Criar `src/workers/usn_watcher.rs`
- [ ] Adicionar feature flags em Cargo.toml
- [ ] Implementar versao condicional de watch_current_folder()
- [ ] Testar deteccao de mudancas
- [ ] Comparar performance com notify
- [ ] Migrar para USN como default (apos validacao)
- [ ] Remover dependencia notify (opcional, pode manter como fallback)

### Fase 4: I/O Priority por Handle
- [ ] Adicionar funcoes em file_flags.rs
- [ ] Adicionar feature `Win32_Storage_FileSystem` se necessario
- [ ] Modificar thumbnail_worker para usar prioridade por handle
- [ ] Testar que operacoes background nao interferem com interativas
- [ ] Verificar que nao ha conflito com THREAD_MODE_BACKGROUND_BEGIN

### Fase 5: MFT Direct Read (⛔ NAO IMPLEMENTAR SEM ORDEM EXPLICITA)
- [ ] **AGUARDAR SOLICITACAO EXPLICITA DO USUARIO**
- [ ] Criar `src/infrastructure/mft_reader.rs`
- [ ] Adicionar funcao `has_admin_privileges()`
- [ ] Implementar `MftReader::read_all_entries()`
- [ ] Implementar `MftReader::build_paths()`
- [ ] Implementar `MftReader::search()`
- [ ] Testar como Administrador
- [ ] Integrar com futura feature de busca global (se desejado)
- [ ] Adicionar fallback para non-admin

---

## Metricas de Validacao

### Testes de Performance

Para cada fase, medir:

1. **Tempo de carregamento de pasta** (1000 arquivos em HDD)
   - Baseline atual: ___ms
   - Apos Fase 1: ___ms
   - Apos Fase 2: ___ms

2. **Responsividade durante scroll** (FPS enquanto carrega thumbnails)
   - Baseline: ___fps
   - Apos otimizacoes: ___fps

3. **Deteccao de mudancas** (tempo para detectar arquivo novo)
   - notify atual: ___ms
   - USN Journal: ___ms

4. **Uso de CPU** (durante idle com pasta aberta)
   - notify: ___%
   - USN Journal: ___%

### Testes de Regressao

- [ ] Verificar que SSD nao e afetado negativamente
- [ ] Verificar que cache funciona corretamente
- [ ] Verificar que invalidacao funciona (criar/deletar/renomear arquivo)
- [ ] Verificar que nao ha memory leaks (monitorar RSS por 1h)

---

## Riscos e Mitigacoes

| Risco | Probabilidade | Impacto | Mitigacao |
|-------|---------------|---------|-----------|
| USN Journal indisponivel (FAT32) | Media | Baixo | Fallback para notify |
| I/O Priority nao suportado (Windows antigo) | Baixa | Baixo | Ignorar erro silenciosamente |
| SQLite lock contention | Baixa | Medio | WAL mode + connection pooling |
| Conflito thread vs handle priority | Media | Baixo | Testar combinacoes antes de deploy |

---

## Ordem de Execucao Recomendada

```
1. [SAFE] FILE_FLAG_SEQUENTIAL_SCAN
   └── Sem risco, beneficio imediato
   └── Impacto: ~15-30% em leitura de imagens

2. [SAFE] Persistent Thumbnail Index
   └── Complementa cache existente
   └── Impacto: ~40-60% em revisitas

3. [MEDIUM] I/O Priority por Handle
   └── Complementa thread priority existente
   └── Impacto: Melhor responsividade durante background work

4. [COMPLEX] USN Journal
   └── Substitui notify (FileSystemWatcher)
   └── Impacto: ~90% menos overhead de monitoramento

5. [⛔ NAO IMPLEMENTAR SEM ORDEM EXPLICITA] MFT Direct Read
   └── AGUARDAR solicitacao explicita do usuario
   └── Requer Administrador, alta complexidade
   └── Usar APENAS para busca global ou indexacao
   └── NAO usar para navegacao de pastas (NtQueryDirectoryFile e suficiente)
   └── Impacto: ~99% mais rapido para enumerar volume inteiro
```

### Arvore de Decisao para MFT

```
Precisa enumerar volume inteiro?
├── NAO → Usar NtQueryDirectoryFile (ja implementado)
└── SIM → Usuario tem privilegios de admin?
          ├── NAO → Usar busca recursiva tradicional
          └── SIM → Drive e NTFS?
                    ├── NAO → Usar busca recursiva tradicional
                    └── SIM → ✅ Usar MFT Direct Read
```

---

## Notas para o Implementador

1. **Sempre testar em HDD real** - Emuladores e VMs nao reproduzem corretamente a latencia de seek

2. **Medir antes de otimizar** - Usar `std::time::Instant` para medir cada operacao

3. **Logs de performance** - Manter os logs `[PERF]` para facilitar debugging

4. **Feature flags** - Usar cargo features para habilitar/desabilitar otimizacoes individualmente

5. **Fallbacks** - Sempre ter fallback para quando APIs nao estao disponiveis

6. **Testar em Windows 10 e 11** - Algumas APIs podem se comportar diferente

7. **⛔ Fase 5 (MFT) e BLOQUEADA** - NAO implementar sem ordem explicita do usuario. As Fases 1-4 cobrem todos os casos de uso normais de um file manager.
