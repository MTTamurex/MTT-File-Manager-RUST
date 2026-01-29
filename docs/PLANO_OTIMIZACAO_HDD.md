# Plano de Implementacao - Otimizacoes para HDD Mecanico

**Data:** 2026-01-29
**Projeto:** MTT File Manager
**Objetivo:** Melhorar a performance de leitura de diretorios em HDDs mecanicos

---

## Resumo Executivo

Este documento detalha um plano de implementacao para otimizar o acesso a HDDs mecanicos no MTT File Manager. As otimizacoes propostas visam reduzir syscalls, minimizar seeks do disco e melhorar a responsividade da UI ao navegar em pastas grandes.

### Otimizacoes Propostas (em ordem de impacto)

| # | Otimizacao | Impacto Estimado | Complexidade |
|---|------------|------------------|--------------|
| 1 | NtQueryDirectoryFile | ~80% reducao em syscalls | Media |
| 2 | Cache de Estrutura de Diretorios | ~50% reducao em I/O repetido | Baixa |
| 3 | Prefetch de Diretorios Adjacentes | ~30% melhora em navegacao | Media |
| 4 | Read-ahead para Thumbnails | ~25% melhora em scroll | Baixa |

---

## 1. NtQueryDirectoryFile - Leitura de Diretorios em Batch

### 1.1 Problema Atual

O codigo atual em `src/app/operations/folder_loading.rs` usa `FindFirstFileW` / `FindNextFileW`:

```rust
// Linha 168-304 em folder_loading.rs
if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
    loop {
        // ... processa 1 arquivo por iteracao
        if FindNextFileW(handle, &mut find_data).is_err() {
            break;
        }
    }
}
```

**Problema:** Cada chamada `FindNextFileW` e uma syscall separada. Para uma pasta com 10.000 arquivos, sao 10.000 syscalls!

### 1.2 Solucao: NtQueryDirectoryFile

`NtQueryDirectoryFile` e a API nativa do Windows (ntdll.dll) que permite ler **multiplas entradas de diretorio em uma unica syscall** usando um buffer grande.

**Vantagens:**
- Buffer de 64KB pode conter ~500-1000 entradas por syscall
- Reducao de ~80-95% no numero de syscalls
- E o que o Windows Explorer usa internamente

### 1.3 Implementacao Detalhada

#### 1.3.1 Adicionar Feature no Cargo.toml

```toml
# Em Cargo.toml, adicionar feature para ntdll
[dependencies.windows]
version = "0.61.0"
features = [
    # ... features existentes ...
    "Win32_System_WindowsProgramming",  # Para NtQueryDirectoryFile
]
```

#### 1.3.2 Criar Novo Modulo: `src/infrastructure/ntfs_reader.rs`

```rust
//! Fast NTFS directory reading using NtQueryDirectoryFile
//!
//! This module provides low-level directory enumeration that reads
//! multiple entries per syscall, dramatically reducing I/O overhead
//! on mechanical HDDs.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, NTSTATUS, STATUS_SUCCESS};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// Buffer size for NtQueryDirectoryFile (64KB is optimal for HDDs)
const BUFFER_SIZE: usize = 65536;

/// Minimum entries to justify using NtQueryDirectoryFile over FindFirstFile
/// For small directories, the overhead isn't worth it
const MIN_ENTRIES_THRESHOLD: usize = 100;

/// Information class for NtQueryDirectoryFile
/// FileDirectoryInformation = 1 (provides name, size, attributes, timestamps)
const FILE_DIRECTORY_INFORMATION: u32 = 1;

/// FileIdBothDirectoryInformation = 37 (includes 8.3 names and file IDs)
/// Use this for even faster enumeration when you don't need 8.3 names
const FILE_ID_BOTH_DIR_INFORMATION: u32 = 37;

/// Raw directory entry from NtQueryDirectoryFile
#[repr(C)]
#[derive(Debug)]
struct FileDirectoryInfo {
    next_entry_offset: u32,
    file_index: u32,
    creation_time: i64,
    last_access_time: i64,
    last_write_time: i64,
    change_time: i64,
    end_of_file: i64,
    allocation_size: i64,
    file_attributes: u32,
    file_name_length: u32,
    // file_name follows (variable length, UTF-16)
}

/// Parsed directory entry
#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,  // Unix timestamp
    pub attributes: u32,
}

/// NtQueryDirectoryFile function signature
type NtQueryDirectoryFileFn = unsafe extern "system" fn(
    file_handle: HANDLE,
    event: HANDLE,
    apc_routine: *mut std::ffi::c_void,
    apc_context: *mut std::ffi::c_void,
    io_status_block: *mut IoStatusBlock,
    file_information: *mut std::ffi::c_void,
    length: u32,
    file_information_class: u32,
    return_single_entry: u8,
    file_name: *const std::ffi::c_void,
    restart_scan: u8,
) -> NTSTATUS;

#[repr(C)]
struct IoStatusBlock {
    status: NTSTATUS,
    information: usize,
}

/// Lazy-loaded NtQueryDirectoryFile function pointer
static NT_QUERY_DIR: std::sync::OnceLock<Option<NtQueryDirectoryFileFn>> = std::sync::OnceLock::new();

fn get_nt_query_directory_file() -> Option<NtQueryDirectoryFileFn> {
    *NT_QUERY_DIR.get_or_init(|| {
        unsafe {
            let ntdll = windows::Win32::System::LibraryLoader::GetModuleHandleW(
                windows::core::w!("ntdll.dll")
            ).ok()?;

            let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
                ntdll,
                windows::core::s!("NtQueryDirectoryFile")
            )?;

            Some(std::mem::transmute(proc))
        }
    })
}

/// Read all entries from a directory using NtQueryDirectoryFile
///
/// # Arguments
/// * `dir_path` - Path to the directory to enumerate
///
/// # Returns
/// Vector of directory entries, or None if the API is unavailable
///
/// # Performance
/// - Uses 64KB buffer to read ~500-1000 entries per syscall
/// - ~80-95% fewer syscalls compared to FindFirstFile/FindNextFile
/// - Best improvement seen on mechanical HDDs
pub fn read_directory_fast(dir_path: &Path) -> Option<Vec<DirectoryEntry>> {
    let nt_query = get_nt_query_directory_file()?;

    // Open directory handle with FILE_LIST_DIRECTORY access
    let dir_wide: Vec<u16> = dir_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(dir_wide.as_ptr()),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,  // Required for directories
            None,
        ).ok()?
    };

    let mut entries = Vec::with_capacity(1000);  // Pre-allocate for typical folder
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut restart_scan = 1u8;  // TRUE for first call

    loop {
        let mut io_status = IoStatusBlock {
            status: NTSTATUS(0),
            information: 0,
        };

        let status = unsafe {
            nt_query(
                handle,
                HANDLE::default(),  // No event
                std::ptr::null_mut(),  // No APC routine
                std::ptr::null_mut(),  // No APC context
                &mut io_status,
                buffer.as_mut_ptr() as *mut _,
                BUFFER_SIZE as u32,
                FILE_DIRECTORY_INFORMATION,
                0,  // Return multiple entries
                std::ptr::null(),  // No file name filter
                restart_scan,
            )
        };

        restart_scan = 0;  // FALSE for subsequent calls

        // STATUS_NO_MORE_FILES = 0x80000006
        if status.0 as u32 == 0x80000006 {
            break;  // End of directory
        }

        if status != STATUS_SUCCESS {
            eprintln!("[NtQuery] Error: 0x{:08X}", status.0 as u32);
            break;
        }

        // Parse entries from buffer
        let mut offset = 0usize;
        loop {
            if offset >= io_status.information {
                break;
            }

            let entry_ptr = unsafe { buffer.as_ptr().add(offset) as *const FileDirectoryInfo };
            let entry = unsafe { &*entry_ptr };

            // Extract filename (UTF-16, variable length)
            let name_ptr = unsafe {
                (entry_ptr as *const u8).add(std::mem::size_of::<FileDirectoryInfo>())
            } as *const u16;

            let name_len = (entry.file_name_length / 2) as usize;
            let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
            let name = OsString::from_wide(name_slice).to_string_lossy().into_owned();

            // Skip . and ..
            if name != "." && name != ".." {
                let is_dir = (entry.file_attributes & 0x10) != 0;  // FILE_ATTRIBUTE_DIRECTORY

                // Convert Windows FILETIME to Unix timestamp
                let modified = if entry.last_write_time > 116444736000000000 {
                    ((entry.last_write_time as u64) - 116444736000000000) / 10_000_000
                } else {
                    0
                };

                entries.push(DirectoryEntry {
                    name,
                    is_dir,
                    size: entry.end_of_file as u64,
                    modified,
                    attributes: entry.file_attributes,
                });
            }

            // Move to next entry
            if entry.next_entry_offset == 0 {
                break;
            }
            offset += entry.next_entry_offset as usize;
        }
    }

    unsafe { let _ = CloseHandle(handle); }

    Some(entries)
}

/// Check if NtQueryDirectoryFile is available on this system
pub fn is_available() -> bool {
    get_nt_query_directory_file().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_directory() {
        if !is_available() {
            eprintln!("NtQueryDirectoryFile not available, skipping test");
            return;
        }

        let entries = read_directory_fast(Path::new("C:\\Windows\\System32"));
        assert!(entries.is_some());

        let entries = entries.unwrap();
        assert!(!entries.is_empty());

        // Should find some known files
        let has_dll = entries.iter().any(|e| e.name.ends_with(".dll"));
        assert!(has_dll, "System32 should have DLL files");
    }
}
```

#### 1.3.3 Modificar `folder_loading.rs` para Usar a Nova API

Alterar a funcao `load_folder()` em `src/app/operations/folder_loading.rs`:

```rust
// No inicio do arquivo, adicionar:
use crate::infrastructure::ntfs_reader;

// Dentro de load_folder(), substituir o loop FindFirstFileW/FindNextFileW por:

// OPTIMIZATION: Try NtQueryDirectoryFile first (much faster on HDDs)
// Falls back to FindFirstFile if unavailable
let use_fast_reader = !is_ssd && ntfs_reader::is_available();

if use_fast_reader {
    eprintln!("[PERF] Using NtQueryDirectoryFile for HDD optimization");

    if let Some(entries) = ntfs_reader::read_directory_fast(&PathBuf::from(&base_path)) {
        for dir_entry in entries {
            // Verifica geracao
            if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                break;
            }

            // Aplica filtros
            let is_hidden = (dir_entry.attributes & 0x02) != 0;  // FILE_ATTRIBUTE_HIDDEN
            let is_system = (dir_entry.attributes & 0x04) != 0;  // FILE_ATTRIBUTE_SYSTEM
            let is_special = matches!(
                dir_entry.name.to_lowercase().as_str(),
                "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
            );

            if !is_hidden && !is_system && !is_special && !dir_entry.name.starts_with('.') {
                let full_path = PathBuf::from(&base_path).join(&dir_entry.name);

                let mut is_dir = dir_entry.is_dir;
                if !is_dir && dir_entry.name.to_lowercase().ends_with(".zip") {
                    is_dir = true;
                }

                let is_onedrive = onedrive::is_onedrive_path(&full_path);
                let sync_status = onedrive::get_sync_status(dir_entry.attributes, is_onedrive);

                let entry = FileEntry {
                    path: full_path,
                    name: dir_entry.name,
                    is_dir,
                    size: if is_dir { 0 } else { dir_entry.size },
                    modified: dir_entry.modified,
                    folder_cover: None,
                    drive_info: None,
                    sync_status,
                    deletion_date: None,
                    recycle_original_path: None,
                };

                batch.push(entry);

                if batch.len() >= batch_size {
                    // ... codigo existente para enviar batch ...
                }
            }
        }

        // ... codigo existente para enviar ultimo batch ...
        return;  // Skip FindFirstFile fallback
    }
}

// Fallback: FindFirstFile/FindNextFile (codigo existente)
// ... resto do codigo atual ...
```

#### 1.3.4 Registrar Modulo

Em `src/infrastructure/mod.rs`, adicionar:

```rust
pub mod ntfs_reader;
```

### 1.4 Testes

```rust
// Em src/infrastructure/ntfs_reader.rs, adicionar mais testes:

#[test]
fn test_performance_comparison() {
    use std::time::Instant;

    let test_path = Path::new("C:\\Windows\\System32");

    // Benchmark NtQueryDirectoryFile
    let start = Instant::now();
    let fast_entries = read_directory_fast(test_path);
    let fast_time = start.elapsed();

    // Benchmark FindFirstFile (simulado)
    let start = Instant::now();
    let mut count = 0;
    // ... usar FindFirstFile aqui ...
    let slow_time = start.elapsed();

    eprintln!("NtQueryDirectoryFile: {:?} ({} entries)", fast_time, fast_entries.map(|e| e.len()).unwrap_or(0));
    eprintln!("FindFirstFile: {:?} ({} entries)", slow_time, count);
}
```

---

## 2. Cache de Estrutura de Diretorios

### 2.1 Problema Atual

Cada vez que o usuario navega para uma pasta, todo o conteudo e lido do disco novamente, mesmo que a pasta tenha sido visitada recentemente.

### 2.2 Solucao: Cache em Memoria

Manter em memoria a estrutura das pastas visitadas recentemente, invalidando apenas quando o FileSystemWatcher detecta mudancas.

### 2.3 Implementacao Detalhada

#### 2.3.1 Criar Novo Modulo: `src/infrastructure/directory_cache.rs`

```rust
//! In-memory directory structure cache
//!
//! Caches directory listings to avoid repeated disk I/O when navigating
//! back to previously visited folders. Uses FileSystemWatcher events
//! for cache invalidation.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use lru::LruCache;
use std::sync::Mutex;
use std::num::NonZeroUsize;

use crate::domain::file_entry::FileEntry;

/// Maximum number of directories to cache
const MAX_CACHED_DIRS: usize = 50;

/// Maximum age before a cached entry is considered stale (5 minutes)
const MAX_CACHE_AGE: Duration = Duration::from_secs(300);

/// Cached directory data
struct CachedDirectory {
    entries: Vec<FileEntry>,
    cached_at: Instant,
    item_count: usize,
}

/// Thread-safe directory cache
pub struct DirectoryCache {
    cache: Mutex<LruCache<PathBuf, CachedDirectory>>,
}

impl DirectoryCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(MAX_CACHED_DIRS).unwrap()
            )),
        }
    }

    /// Try to get cached entries for a directory
    ///
    /// Returns None if:
    /// - Directory is not in cache
    /// - Cache entry is older than MAX_CACHE_AGE
    pub fn get(&self, path: &PathBuf) -> Option<Vec<FileEntry>> {
        let mut cache = self.cache.lock().ok()?;

        if let Some(cached) = cache.get(path) {
            // Check if cache is still fresh
            if cached.cached_at.elapsed() < MAX_CACHE_AGE {
                eprintln!("[DirCache] HIT: {:?} ({} items)", path, cached.item_count);
                return Some(cached.entries.clone());
            } else {
                eprintln!("[DirCache] STALE: {:?}", path);
            }
        }

        None
    }

    /// Cache directory entries
    pub fn put(&self, path: PathBuf, entries: Vec<FileEntry>) {
        if let Ok(mut cache) = self.cache.lock() {
            let item_count = entries.len();
            cache.put(path.clone(), CachedDirectory {
                entries,
                cached_at: Instant::now(),
                item_count,
            });
            eprintln!("[DirCache] STORE: {:?} ({} items)", path, item_count);
        }
    }

    /// Invalidate cache for a specific directory
    ///
    /// Called when FileSystemWatcher detects changes
    pub fn invalidate(&self, path: &PathBuf) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache.pop(path).is_some() {
                eprintln!("[DirCache] INVALIDATE: {:?}", path);
            }
        }
    }

    /// Invalidate all entries that are children of a path
    ///
    /// Used when a parent directory changes (rename, delete)
    pub fn invalidate_children(&self, parent: &PathBuf) {
        if let Ok(mut cache) = self.cache.lock() {
            let keys_to_remove: Vec<PathBuf> = cache
                .iter()
                .filter(|(k, _)| k.starts_with(parent))
                .map(|(k, _)| k.clone())
                .collect();

            for key in keys_to_remove {
                cache.pop(&key);
                eprintln!("[DirCache] INVALIDATE (child): {:?}", key);
            }
        }
    }

    /// Clear entire cache
    ///
    /// Called on manual refresh (F5)
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.clear();
            eprintln!("[DirCache] CLEAR ALL");
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> (usize, usize) {
        if let Ok(cache) = self.cache.lock() {
            let total_items: usize = cache.iter().map(|(_, v)| v.item_count).sum();
            (cache.len(), total_items)
        } else {
            (0, 0)
        }
    }
}

impl Default for DirectoryCache {
    fn default() -> Self {
        Self::new()
    }
}
```

#### 2.3.2 Integrar com `ImageViewerApp`

Em `src/app/state.rs`, adicionar o cache:

```rust
use crate::infrastructure::directory_cache::DirectoryCache;

pub struct ImageViewerApp {
    // ... campos existentes ...

    /// Cache de estrutura de diretorios para navegacao rapida
    pub directory_cache: Arc<DirectoryCache>,
}

impl ImageViewerApp {
    pub fn new(/* ... */) -> Self {
        Self {
            // ... inicializacao existente ...
            directory_cache: Arc::new(DirectoryCache::new()),
        }
    }
}
```

#### 2.3.3 Usar Cache em `load_folder()`

Modificar `src/app/operations/folder_loading.rs`:

```rust
pub fn load_folder(&mut self, force_refresh: bool) {
    // ... codigo existente de limpeza ...

    // Se force_refresh, limpa o cache
    if force_refresh {
        self.directory_cache.clear();
        // ... resto do codigo de limpeza ...
    }

    let directory_cache = self.directory_cache.clone();
    let current_path_for_cache = self.current_path.clone();

    std::thread::spawn(move || {
        // OPTIMIZATION: Check directory cache first
        if !force_refresh {
            if let Some(cached_entries) = directory_cache.get(&PathBuf::from(&current_path)) {
                eprintln!("[PERF] Using cached directory listing");

                // Send cached entries in batches
                for chunk in cached_entries.chunks(batch_size) {
                    if gen_clone.load(AtomicOrdering::Relaxed) != my_gen {
                        return;
                    }
                    let _ = file_entry_sender.send((my_gen, chunk.to_vec()));
                    ctx.request_repaint();
                }

                // Signal completion
                let _ = file_entry_sender.send((my_gen, Vec::new()));
                ctx.request_repaint();
                return;
            }
        }

        // ... codigo existente de leitura do disco ...

        // Apos ler do disco, armazena no cache
        // (precisa coletar todas as entries primeiro)
        let all_entries: Vec<FileEntry> = /* coletar do batch */;
        directory_cache.put(PathBuf::from(&current_path_for_cache), all_entries);
    });
}
```

#### 2.3.4 Integrar com FileSystemWatcher

Em `src/app/operations/file_watcher.rs`, adicionar invalidacao:

```rust
// Quando detectar mudanca em um diretorio:
fn handle_fs_event(&mut self, event: notify::Event) {
    match event.kind {
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(_) => {
            for path in event.paths {
                if let Some(parent) = path.parent() {
                    self.directory_cache.invalidate(&parent.to_path_buf());
                }
            }
        }
        EventKind::Remove(RemoveKind::Folder) => {
            for path in event.paths {
                self.directory_cache.invalidate_children(&path);
            }
        }
        _ => {}
    }
}
```

---

## 3. Prefetch de Diretorios Adjacentes

### 3.1 Problema Atual

Quando o usuario navega para uma pasta, ele frequentemente entra em subdiretorios logo em seguida. Atualmente, cada navegacao dispara uma nova leitura do disco.

### 3.2 Solucao: Prefetch em Background

Apos carregar um diretorio, iniciar leitura em background dos subdiretorios visiveis.

### 3.3 Implementacao Detalhada

#### 3.3.1 Criar Worker de Prefetch

Adicionar em `src/workers/prefetch_worker.rs`:

```rust
//! Directory prefetch worker
//!
//! Proactively loads directory contents in background to speed up navigation.
//! Only runs on HDDs where seek latency makes prefetching worthwhile.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::ntfs_reader;
use crate::domain::file_entry::FileEntry;

/// Maximum subdirectories to prefetch per navigation
const MAX_PREFETCH_DIRS: usize = 5;

/// Message to prefetch worker
pub enum PrefetchMessage {
    /// Prefetch these directories
    Prefetch(Vec<PathBuf>),
    /// Shutdown worker
    Shutdown,
}

/// Spawn the prefetch worker thread
pub fn spawn_prefetch_worker(
    receiver: Receiver<PrefetchMessage>,
    directory_cache: Arc<DirectoryCache>,
) {
    std::thread::spawn(move || {
        // Set lowest priority - prefetch should never interfere with user actions
        io_priority::set_thread_priority(IOPriority::Background);

        while let Ok(msg) = receiver.recv() {
            match msg {
                PrefetchMessage::Prefetch(paths) => {
                    for path in paths.into_iter().take(MAX_PREFETCH_DIRS) {
                        // Skip if already cached
                        if directory_cache.get(&path).is_some() {
                            continue;
                        }

                        eprintln!("[Prefetch] Loading: {:?}", path);

                        // Use fast reader if available
                        if let Some(entries) = ntfs_reader::read_directory_fast(&path) {
                            let file_entries: Vec<FileEntry> = entries
                                .into_iter()
                                .filter(|e| {
                                    let is_hidden = (e.attributes & 0x02) != 0;
                                    let is_system = (e.attributes & 0x04) != 0;
                                    !is_hidden && !is_system && !e.name.starts_with('.')
                                })
                                .map(|e| FileEntry {
                                    path: path.join(&e.name),
                                    name: e.name,
                                    is_dir: e.is_dir,
                                    size: if e.is_dir { 0 } else { e.size },
                                    modified: e.modified,
                                    folder_cover: None,
                                    drive_info: None,
                                    sync_status: crate::domain::file_entry::SyncStatus::None,
                                    deletion_date: None,
                                    recycle_original_path: None,
                                })
                                .collect();

                            directory_cache.put(path, file_entries);
                        }
                    }
                }
                PrefetchMessage::Shutdown => break,
            }
        }

        io_priority::reset_thread_priority();
    });
}
```

#### 3.3.2 Integrar com `load_folder()`

Apos carregar um diretorio, enviar subdiretorios para prefetch:

```rust
// Em folder_loading.rs, apos enviar o ultimo batch:

// OPTIMIZATION: Prefetch visible subdirectories (HDD only)
if !is_ssd {
    let subdirs: Vec<PathBuf> = all_entries
        .iter()
        .filter(|e| e.is_dir)
        .take(5)  // Apenas os primeiros 5 subdiretorios
        .map(|e| e.path.clone())
        .collect();

    if !subdirs.is_empty() {
        let _ = prefetch_sender.send(PrefetchMessage::Prefetch(subdirs));
    }
}
```

---

## 4. Read-ahead para Thumbnails

### 4.1 Problema Atual

O sistema de thumbnails ja agrupa por diretorio (`DirectoryGroupedQueue` em `io_priority.rs`), mas nao preve a direcao do scroll do usuario.

### 4.2 Solucao: Scroll Direction Prediction

Detectar a direcao do scroll e pre-carregar thumbnails "adiante".

### 4.3 Implementacao Detalhada

#### 4.3.1 Modificar `request_thumbnail_load()`

Em `src/ui/thumbnails.rs` ou onde thumbnails sao requisitados:

```rust
/// Track scroll direction for predictive prefetch
pub struct ScrollPredictor {
    last_visible_start: usize,
    last_visible_end: usize,
    scroll_direction: ScrollDirection,
    velocity: f32,  // Items per frame
}

#[derive(Clone, Copy, PartialEq)]
pub enum ScrollDirection {
    None,
    Down,
    Up,
}

impl ScrollPredictor {
    pub fn new() -> Self {
        Self {
            last_visible_start: 0,
            last_visible_end: 0,
            scroll_direction: ScrollDirection::None,
            velocity: 0.0,
        }
    }

    /// Update predictor with current visible range
    pub fn update(&mut self, visible_start: usize, visible_end: usize) {
        if visible_start > self.last_visible_start {
            self.scroll_direction = ScrollDirection::Down;
            self.velocity = (visible_start - self.last_visible_start) as f32;
        } else if visible_start < self.last_visible_start {
            self.scroll_direction = ScrollDirection::Up;
            self.velocity = (self.last_visible_start - visible_start) as f32;
        } else {
            self.velocity *= 0.9;  // Decay
            if self.velocity < 0.5 {
                self.scroll_direction = ScrollDirection::None;
            }
        }

        self.last_visible_start = visible_start;
        self.last_visible_end = visible_end;
    }

    /// Get prefetch range based on scroll direction
    pub fn get_prefetch_range(&self, total_items: usize) -> (usize, usize) {
        let prefetch_count = 20;  // Items to prefetch ahead

        match self.scroll_direction {
            ScrollDirection::Down => {
                let start = self.last_visible_end;
                let end = (start + prefetch_count).min(total_items);
                (start, end)
            }
            ScrollDirection::Up => {
                let end = self.last_visible_start;
                let start = end.saturating_sub(prefetch_count);
                (start, end)
            }
            ScrollDirection::None => {
                // Prefetch in both directions
                let mid = (self.last_visible_start + self.last_visible_end) / 2;
                let start = mid.saturating_sub(prefetch_count / 2);
                let end = (mid + prefetch_count / 2).min(total_items);
                (start, end)
            }
        }
    }
}
```

#### 4.3.2 Usar Predictor no Loop de Render

```rust
// Em grid_view.rs ou onde thumbnails sao renderizados:

// Apos renderizar itens visiveis:
scroll_predictor.update(first_visible_index, last_visible_index);

// Prefetch na direcao do scroll
let (prefetch_start, prefetch_end) = scroll_predictor.get_prefetch_range(items.len());
for i in prefetch_start..prefetch_end {
    if !is_thumbnail_cached(&items[i].path) {
        request_thumbnail_load(
            &items[i].path,
            generation,
            thumbnail_size,
            IOPriority::Prefetch,  // Lower priority than visible items
        );
    }
}
```

---

## 5. Checklist de Implementacao

### Fase 1: NtQueryDirectoryFile (Maior Impacto)
- [ ] Criar `src/infrastructure/ntfs_reader.rs`
- [ ] Adicionar feature `Win32_System_WindowsProgramming` no Cargo.toml
- [ ] Registrar modulo em `src/infrastructure/mod.rs`
- [ ] Modificar `load_folder()` para usar a nova API
- [ ] Adicionar testes unitarios
- [ ] Testar em HDD real com pasta grande (10k+ arquivos)

### Fase 2: Cache de Diretorios
- [ ] Criar `src/infrastructure/directory_cache.rs`
- [ ] Adicionar campo `directory_cache` em `ImageViewerApp`
- [ ] Integrar cache em `load_folder()`
- [ ] Integrar invalidacao com FileSystemWatcher
- [ ] Testar navegacao back/forward

### Fase 3: Prefetch de Subdiretorios
- [ ] Criar `src/workers/prefetch_worker.rs`
- [ ] Adicionar channel de comunicacao em `ImageViewerApp`
- [ ] Spawn worker na inicializacao
- [ ] Enviar subdiretorios para prefetch apos load_folder()
- [ ] Testar impacto na navegacao

### Fase 4: Read-ahead para Thumbnails
- [ ] Criar `ScrollPredictor` struct
- [ ] Integrar predictor no loop de render
- [ ] Ajustar prioridades de prefetch
- [ ] Testar scrolling em pasta com muitos arquivos

---

## 6. Metricas de Sucesso

### Antes da Implementacao (medir baseline)
- [ ] Tempo para carregar pasta com 1.000 arquivos em HDD
- [ ] Tempo para carregar pasta com 10.000 arquivos em HDD
- [ ] Tempo para navegar back para pasta ja visitada
- [ ] FPS durante scroll em pasta grande

### Apos Implementacao (comparar)
- [ ] Mesmas metricas acima
- [ ] Reducao esperada: 50-80% no tempo de carregamento
- [ ] Cache hit rate esperado: >80% para navegacao back/forward

---

## 7. Riscos e Mitigacoes

| Risco | Mitigacao |
|-------|-----------|
| NtQueryDirectoryFile nao disponivel | Fallback para FindFirstFile ja implementado |
| Cache consome muita memoria | LRU com limite de 50 diretorios |
| Prefetch interfere com operacoes do usuario | Prioridade Background + yield frequente |
| Cache desatualizado | Integracao com FileSystemWatcher para invalidacao |

---

## 8. Recursos Adicionais

### Documentacao de Referencia
- [NtQueryDirectoryFile - MSDN](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntquerydirectoryfile)
- [FILE_DIRECTORY_INFORMATION structure](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_directory_information)

### Codigo de Referencia
- Windows Explorer: Usa NtQueryDirectoryFile internamente
- Everything (voidtools): Leitura direta da MFT para velocidade maxima
- Total Commander: Cache agressivo de diretorios

---

## Notas Finais

Este plano foi criado com base na analise do codigo atual do projeto. As implementacoes sugeridas sao incrementais e podem ser feitas em fases separadas. A Fase 1 (NtQueryDirectoryFile) deve trazer o maior impacto imediato e e recomendada como primeira implementacao.

Para duvidas ou esclarecimentos, consulte os arquivos de codigo mencionados neste documento.
