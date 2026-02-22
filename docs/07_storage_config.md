# Configuração e Storage - MTT File Manager

## Objetivo do Documento
Este documento descreve onde e como o MTT File Manager armazena configurações, cache, estado e como gerenciar esses dados.

## Localização dos Dados

O MTT File Manager armazena dados em dois diretórios distintos:

### Diretório do App (por usuário)
```
%LOCALAPPDATA%\MTT-File-Manager\
```

Exemplo típico:
```
C:\Users\Username\AppData\Local\MTT-File-Manager\
```

### Diretório do Serviço de Busca (compartilhado)
```
%PROGRAMDATA%\MTT-File-Manager\
```

Exemplo típico:
```
C:\ProgramData\MTT-File-Manager\
```

### Estrutura de Diretórios
```
%LOCALAPPDATA%\MTT-File-Manager/          # Dados do app (por usuário)
├── thumbnails/           # Cache de thumbnails
│   ├── thumbnails.db    # Banco SQLite principal
│   └── *.webp          # Arquivos de thumbnail individuais
└── virtual_drive_config.json  # Config de drives virtuais

%PROGRAMDATA%\MTT-File-Manager/            # Dados do serviço de busca (global)
└── search_index.db      # Índice de arquivos do serviço de busca
```

## Banco de Dados SQLite

### Schema Principal
**Arquivo**: `thumbnails/thumbnails.db`

**Tabelas**:
```sql
-- Thumbnails (Cache de thumbnails)
CREATE TABLE thumbnails (
    path_hash INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL,
    thumbnail_path TEXT,
    file_size INTEGER,
    modified_time INTEGER,
    width INTEGER,
    height INTEGER,
    created_at INTEGER DEFAULT CURRENT_TIMESTAMP
);

-- Preferences (Preferências do usuário)
CREATE TABLE preferences (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER DEFAULT CURRENT_TIMESTAMP
);

-- Directory cache (Cache de diretórios)
CREATE TABLE directory_cache (
    path TEXT PRIMARY KEY,
    file_count INTEGER,
    total_size INTEGER,
    cached_at INTEGER DEFAULT CURRENT_TIMESTAMP
);

-- Folder locks (preferências de view por pasta — persiste view_mode, sort, etc.)
CREATE TABLE folder_locks (
    path TEXT PRIMARY KEY,
    view_mode TEXT NOT NULL,
    sort_mode TEXT NOT NULL,
    sort_descending TEXT NOT NULL,
    folders_position TEXT NOT NULL
);

-- Quick Access — pastas fixadas na sidebar pelo usuário
CREATE TABLE pinned_folders (
    path TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0
);

-- Folder previews — covers customizados de pastas (composição back + thumbnail + front)
CREATE TABLE folder_previews (
    path_hash INTEGER PRIMARY KEY,
    file_path TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    image_data BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);
```

### Acesso ao Banco
**Código**: `src/infrastructure/disk_cache.rs`

```rust
pub struct ThumbnailDiskCache {
    writer: Arc<Mutex<Connection>>,
    reader: Arc<Mutex<Connection>>,
    cache_dir: PathBuf,
}
```

### Operações Principais
```rust
// Inicializar cache
let disk_cache = Arc::new(ThumbnailDiskCache::new()?);

// Salvar thumbnail
disk_cache.set_thumbnail(path, thumbnail_data, width, height)?;

// Recuperar thumbnail
let thumbnail = disk_cache.get_thumbnail(path)?;

// Salvar preferência
disk_cache.set_preference("sort_mode", "name")?;

// Recuperar preferência
let value = disk_cache.get_preference("sort_mode");
```

## Configurações (Preferences)

### Preferências Armazenadas

**Local de carregamento**: `src/app/init.rs` - `ImageViewerApp::new()`

```rust
// Carrega preferências do SQLite
let sort_mode = disk_cache
    .get_preference("sort_mode")
    .map(|s| match s.as_str() {
        "date" => SortMode::Date,
        "size" => SortMode::Size,
        "type" => SortMode::Type,
        "drive_total" => SortMode::DriveTotalSpace,
        "drive_free" => SortMode::DriveFreeSpace,
        _ => SortMode::Name,
    })
    .unwrap_or(SortMode::Name);

let thumbnail_size = disk_cache
    .get_preference("thumbnail_size")
    .and_then(|s| s.parse::<f32>().ok())
    .unwrap_or(128.0);

let show_preview_panel = disk_cache
    .get_preference("show_preview_panel")
    .map(|s| s != "false")
    .unwrap_or(true);
```

### Lista Completa de Preferências

| Chave | Tipo | Padrão | Descrição |
|-------|------|--------|-----------|
| `sort_mode` | String | "name" | Modo de ordenação (name, date, size, type, drive_total, drive_free) |
| `sort_descending` | Bool | false | Ordenação descendente |
| `sort_mode_computer` | String | "name" | Sort mode para view "Este Computador" |
| `sort_mode_normal` | String | "name" | Sort mode para views normais |
| `folders_position` | String | "first" | Posição de pastas (first, last, mixed) |
| `thumbnail_size` | Float | 128.0 | Tamanho dos thumbnails (64-512) |
| `view_mode` | String | "grid" | Modo de visualização (grid, list) |
| `show_preview_panel` | Bool | true | Mostrar painel de preview |
| `window_width` | Float | 1280.0 | Largura da janela |
| `window_height` | Float | 720.0 | Altura da janela |
| `window_is_maximized` | Bool | false | Janela maximizada |
| `sidebar_left_width` | Float | 200.0 | Largura sidebar esquerda |
| `sidebar_right_width` | Float | 300.0 | Largura sidebar direita (preview panel) |

### Código de Salvamento
**Local**: `src/app/operations/preferences.rs`

```rust
// Salvar preferência
pub fn save_preference(&self, key: &str, value: &str) {
    if let Err(e) = self.disk_cache.set_preference(key, value) {
        eprintln!("[PREFS] Failed to save {}: {}", key, e);
    }
}

// Exemplo de uso
app.save_preference("sort_mode", "date");
app.save_preference("thumbnail_size", "256.0");
```

## Cache de Thumbnails

### Formato dos Arquivos
- **Formato**: WebP (compressão lossy com qualidade configurável)
- **Qualidade**: Padrão 75%
- **Tamanho**: Baseado em `thumbnail_size` setting (64-512px)
- **Local**: `thumbnails/*.webp`
- **Hash**: Nome do arquivo é hash do path (FxHash)

### Estrutura de Cache
```rust
// src/infrastructure/disk_cache.rs
pub fn set_thumbnail(
    &self,
    path: &Path,
    image_data: &[u8],
    width: u32,
    height: u32,
) -> Result<PathBuf> {
    // Gera hash único do path
    let path_hash = hash_path(path);
    
    // Cria filename baseado no hash
    let thumbnail_filename = format!("{}.webp", path_hash);
    let thumbnail_path = self.cache_dir.join(&thumbnail_filename);
    
    // Salva com compressão WebP
    let encoder = webp::Encoder::from_image(&img)?;
    let encoded = encoder.encode(quality);
    fs::write(&thumbnail_path, encoded.as_bytes())?;
    
    // Atualiza registro no SQLite
    self.update_thumbnail_record(path, path_hash, &thumbnail_path, ...)?;
    
    Ok(thumbnail_path)
}
```

### Invalidação de Cache
```rust
// Verifica se thumbnail está desatualizado
pub fn is_thumbnail_stale(
    &self,
    path: &Path,
    current_size: u64,
    current_modified: u64,
) -> bool {
    // Compara com dados armazenados no SQLite
    if let Some((stored_size, stored_modified)) = self.get_thumbnail_info(path) {
        stored_size != current_size || stored_modified != current_modified
    } else {
        true // Não existe no cache
    }
}
```

## Cache em Memória

### Texture Cache
**Local**: `src/ui/cache.rs`

```rust
pub struct CacheManager {
    pub texture_cache: DashMap<PathBuf, egui::TextureHandle>,
    pub icon_cache: DashMap<PathBuf, egui::TextureHandle>,
    pub loading_thumbnails: FxHashSet<PathBuf>,
    pub failed_thumbnails: FxHashSet<PathBuf>,
    pub loading_icons: FxHashSet<PathBuf>,
    pub failed_icons: FxHashSet<PathBuf>,
}
```

### Directory Cache
**Local**: `src/infrastructure/directory_cache.rs`

```rust
pub struct DirectoryCache {
    cache: DashMap<PathBuf, CachedDirectory>,
}

struct CachedDirectory {
    entries: Vec<FileEntry>,
    cached_at: Instant,
}
```

### Directory Index
**Local**: `src/infrastructure/directory_index.rs`

```rust
pub struct DirectoryIndex {
    // Índice de arquivos para busca rápida
    path_to_index: HashMap<PathBuf, usize>,
    all_files: Vec<FileEntry>,
}
```

## Banco de Dados do Serviço de Busca

### Schema
**Arquivo**: `%PROGRAMDATA%\MTT-File-Manager\search_index.db`
**Código**: `crates/mtt-search-service/src/index_db.rs`

```sql
-- Estado de cada volume indexado
CREATE TABLE volume_state (
    drive_letter TEXT PRIMARY KEY,
    journal_id INTEGER NOT NULL,     -- USN Journal ID (para detectar resets)
    last_usn INTEGER NOT NULL,       -- Último USN processado
    files_indexed INTEGER NOT NULL,  -- Número total de registros
    last_full_scan_epoch INTEGER NOT NULL  -- Timestamp do último full scan
);

-- Registros de arquivos (índice persistido)
CREATE TABLE file_records (
    frn INTEGER NOT NULL,            -- FRN NTFS ou referência sintética (volumes sem USN)
    drive_letter TEXT NOT NULL,
    name TEXT NOT NULL,              -- Nome do arquivo/pasta
    parent_frn INTEGER NOT NULL,     -- FRN do diretório pai
    is_dir INTEGER NOT NULL,
    PRIMARY KEY (drive_letter, frn)
);
```

### Configuração SQLite
- **Modo**: WAL (Write-Ahead Logging) para melhor concorrência
- **Synchronous**: NORMAL (trade-off entre performance e durabilidade)

### Fluxo de Startup
1. Serviço abre/cria `search_index.db`
2. Para cada volume detectado (`discover_volumes`):
   - Se `usn_supported` (`NTFS`/`ReFS`):
     - Carrega `volume_state` e valida `journal_id`
     - Se válido: `load_into_index()` + catch-up incremental via USN Journal
     - Se inválido/ausente: full re-scan do MFT (`FSCTL_ENUM_USN_DATA`)
     - Persistência periódica a cada 5 minutos
   - Se **sem USN** (exFAT/FAT32/FUSE/CryptoFS etc.):
     - Tenta `load_into_index()` para disponibilizar cache imediatamente
     - Executa full scan com `fs_walker::scan_volume()`
     - Persiste após cada scan completo
     - Re-scan periódico (30s para fuse/cryptofs/dokan/winfsp, 120s para demais)

**Nota**: em volumes sem USN, `journal_id` e `last_usn` são persistidos como `0`.

### Acesso ao Banco
```rust
pub struct IndexDb {
    conn: Mutex<Connection>,
}

// Carregar estado do volume
pub fn load_volume_state(&self, drive_letter: char) -> Option<PersistedVolumeState>

// Carregar registros de arquivos
pub fn load_into_index(&self, index: &mut VolumeIndex) -> Option<usize>

// Salvar índice completo
pub fn save_volume(&self, index: &VolumeIndex) -> Result<(), String>
```

### Limpar Índice do Serviço de Busca
```powershell
# Parar o serviço antes
sc.exe stop MTTFileManagerSearch

# Remover banco do índice (será recriado com full scan)
Remove-Item "$env:PROGRAMDATA\MTT-File-Manager\search_index.db" -Force

# Reiniciar serviço
sc.exe start MTTFileManagerSearch
```

## Configuração de Drives Virtuais

### Arquivo de Configuração
**Local**: `virtual_drive_config.json`

**Formato**:
```json
{
    "virtual_drives": [
        {
            "name": "ISO_Mount",
            "path": "C:\\ISOs",
            "enabled": true
        }
    ]
}
```

**Código**: `src/infrastructure/virtual_drive_config.rs`

```rust
pub struct VirtualDriveConfig {
    pub virtual_drives: Vec<VirtualDrive>,
}

pub struct VirtualDrive {
    pub name: String,
    pub path: PathBuf,
    pub enabled: bool,
}
```

## Como Limpar/Resetar Dados

### Limpar Cache de Thumbnails
```powershell
# Remove todo o diretório de cache
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Ou apenas os arquivos WebP (mantém preferências)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.webp"
```

### Limpar Banco de Dados
```powershell
# Remove apenas o banco (mantém arquivos WebP, mas invalida)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db"
```

### Resetar Preferências
```powershell
# Abre o banco SQLite e remove preferências
# Ou delete o arquivo .db (recria com defaults)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db"
```

### Limpar Tudo (Fresh Start)
```powershell
# Remove cache e preferências do app
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Remove config de drives virtuais
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\virtual_drive_config.json"

# Remove índice do serviço de busca (requer admin)
sc.exe stop MTTFileManagerSearch
Remove-Item "$env:PROGRAMDATA\MTT-File-Manager" -Recurse -Force
sc.exe start MTTFileManagerSearch
```

## Migração de Dados

### Backup de Configurações
```powershell
# Backup completo
$source = "$env:LOCALAPPDATA\MTT-File-Manager"
$dest = "$env:USERPROFILE\Desktop\MTT-Backup"
Copy-Item $source $dest -Recurse
```

### Restaurar Configurações
```powershell
# Restaurar
$source = "$env:USERPROFILE\Desktop\MTT-Backup\MTT-File-Manager"
$dest = "$env:LOCALAPPDATA\MTT-File-Manager"
Copy-Item $source $dest -Recurse -Force
```

## Troubleshooting de Storage

### Erro "Database is locked"
- **Causa**: Múltiplas instâncias tentando acessar o SQLite
- **Solução**: Fechar outras instâncias, ou usar modo WAL (já implementado)

### Cache não persiste
- **Causa**: Sem permissão de escrita em %LOCALAPPDATA%
- **Debug**: Verificar se diretório existe e é gravável
- **Solução**: Executar como administrador uma vez para criar diretório

### Thumbnails duplicados
- **Causa**: Hash collision ou path normalizado diferente
- **Solução**: Limpar cache, verificar normalização de paths

---

*Última atualização: 2026-02-22 (adicionada tabela folder_previews para covers customizados de pastas)*
