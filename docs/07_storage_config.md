# Configuração e Storage - MTT File Manager

## Objetivo do Documento
Este documento descreve onde e como o MTT File Manager armazena configurações, cache, estado e como gerenciar esses dados.

## Localização dos Dados

### Diretório Base
```
%LOCALAPPDATA%\MTT-File-Manager\
```
Exemplo típico:
```
C:\Users\Username\AppData\Local\MTT-File-Manager\
```

### Estrutura de Diretórios
```
MTT-File-Manager/
├── thumbnails/           # Cache de thumbnails
│   ├── thumbnails.db    # Banco SQLite principal
│   └── *.webp          # Arquivos de thumbnail individuais
└── virtual_drive_config.json  # Config de drives virtuais
```

## Banco de Dados SQLite

### Schema Principal
**Arquivo**: `thumbnails/thumbnails.db`

**Tabelas**:
```sql
-- Thumbnails
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

-- Preferences
CREATE TABLE preferences (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER DEFAULT CURRENT_TIMESTAMP
);

-- Directory cache
CREATE TABLE directory_cache (
    path TEXT PRIMARY KEY,
    file_count INTEGER,
    total_size INTEGER,
    cached_at INTEGER DEFAULT CURRENT_TIMESTAMP
);
```

### Acesso ao Banco
**Código**: `src/infrastructure/disk_cache.rs`

```rust
pub struct ThumbnailDiskCache {
    writer: Arc<Mutex<Connection>>,
    reader: Arc<Mutex<Connection>>,
}
```

### Operações Principais
```rust
// Salvar thumbnail
disk_cache.set_thumbnail(path, thumbnail_data, width, height);

// Recuperar thumbnail
disk_cache.get_thumbnail(path);

// Salvar preferência
disk_cache.set_preference(key, value);

// Recuperar preferência
disk_cache.get_preference(key);
```

## Configurações (Preferences)

### Preferências Armazenadas
```rust
// src/app/init.rs - Carregamento
let sort_mode = disk_cache
    .get_preference("sort_mode")
    .unwrap_or("name".to_string());

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
| `sort_mode` | String | "name" | Modo de ordenação (name, date, size, type) |
| `sort_descending` | Bool | false | Ordenação descendente |
| `folders_position` | String | "first" | Posição de pastas (first, last, mixed) |
| `thumbnail_size` | Float | 128.0 | Tamanho dos thumbnails (64-512) |
| `view_mode` | String | "grid" | Modo de visualização (grid, list) |
| `show_preview_panel` | Bool | true | Mostrar painel de preview |
| `window_width` | Float | 1280.0 | Largura da janela |
| `window_height` | Float | 720.0 | Altura da janela |
| `window_is_maximized` | Bool | true | Janela maximizada |
| `sidebar_left_width` | Float | 200.0 | Largura sidebar esquerda |
| `sidebar_right_width` | Float | 300.0 | Largura sidebar direita |
| `upload_budget_ms` | Float | 6.0 | Budget de upload GPU (2-10) |

### Código de Carregamento
**Local**: `src/app/init.rs` - `ImageViewerApp::new()`

```rust
// Load Preferences from SQLite
let sort_mode = disk_cache
    .get_preference("sort_mode")
    .map(|s| match s.as_str() {
        "date" => SortMode::Date,
        "size" => SortMode::Size,
        "type" => SortMode::Type,
        _ => SortMode::Name,
    })
    .unwrap_or(SortMode::Name);
```

## Cache de Thumbnails

### Formato dos Arquivos
- **Formato**: WebP (compressão lossy)
- **Qualidade**: Configurável (padrão: 75%)
- **Tamanho**: Baseado em `thumbnail_size` setting
- **Local**: `thumbnails/*.webp`

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
    webp::Encoder::from_image(&img)
        .encode(quality)
        .as_bytes()
}
```

### Invalidação de Cache
```rust
// Verifica se thumbnail está desatualizado
pub fn is_thumbnail_stale(
    &self,
    path: &Path,
    current_size: u64,
    current_modified: SystemTime,
) -> bool {
    // Compara com dados armazenados no SQLite
    stored_size != current_size || stored_modified != current_modified
}
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

## Cache de Diretórios

### Implementação
**Código**: `src/infrastructure/directory_cache.rs`

```rust
pub struct DirectoryCache {
    cache: Arc<DashMap<PathBuf, CachedDirectory>>,
}

pub struct CachedDirectory {
    pub entries: Vec<FileEntry>,
    pub file_count: usize,
    pub total_size: u64,
    pub cached_at: Instant,
}
```

### TTL (Time To Live)
```rust
const CACHE_TTL: Duration = Duration::from_secs(30);

impl DirectoryCache {
    pub fn is_expired(&self, path: &Path) -> bool {
        if let Some(cached) = self.cache.get(path) {
            cached.cached_at.elapsed() > CACHE_TTL
        } else {
            true
        }
    }
}
```

## Cache de Índice

### Implementação
**Código**: `src/infrastructure/directory_index.rs`

```rust
pub struct DirectoryIndex {
    conn: Connection,
}

// Índice para acelerar buscas por path
CREATE INDEX idx_thumbnails_path ON thumbnails(file_path);
CREATE INDEX idx_thumbnails_hash ON thumbnails(path_hash);
```

## Migrações e Versionamento

### Migração de Legacy
```rust
// src/infrastructure/disk_cache.rs
fn cleanup_legacy(cache_dir: &Path) {
    // Remove arquivos antigos de cache (se existirem)
    let legacy_files = ["thumbnails.cache", "icons.cache"];
    for file in &legacy_files {
        let path = cache_dir.join(file);
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
}
```

### Schema Versioning
```rust
const CURRENT_SCHEMA_VERSION: i32 = 1;

fn check_schema_version(conn: &Connection) -> Result<i32> {
    // Verifica versão atual do schema
    // Aplica migrações se necessário
}
```

## Gerenciamento de Storage

### Tamanho do Cache
```rust
impl ThumbnailDiskCache {
    pub fn get_cache_size(&self) -> Result<u64> {
        // Calcula tamanho total dos arquivos .webp
        let mut total_size = 0u64;
        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some("webp") {
                total_size += entry.metadata()?.len();
            }
        }
        Ok(total_size)
    }
}
```

### Garbage Collection
```rust
pub fn garbage_collect(&self) -> Result<usize> {
    // Remove thumbnails órfãos
    // Remove preferências inválidas
    // Vacuum do banco de dados
}
```

## Como Resetar/Limpar Dados

### Método 1: Via Interface (quando implementado)
```rust
// Futuro: menu de configurações
app.clear_cache();
app.reset_preferences();
```

### Método 2: Manual
```powershell
# Parar o aplicativo
# Deletar diretório de cache completo
Remove-Item -Path "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Ou apenas partes específicas
Remove-Item -Path "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.webp"
Remove-Item -Path "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db"
```

### Método 3: Backup e Restore
```powershell
# Backup
Copy-Item "$env:LOCALAPPDATA\MTT-File-Manager" "C:\Backup\MTT-File-Manager-Backup"

# Restore
Copy-Item "C:\Backup\MTT-File-Manager-Backup" "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

## Debugging de Storage

### Verificar Integridade do Banco
```powershell
# Instalar SQLite CLI (se não tiver)
# Verificar integridade do banco
sqlite3 "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db" "PRAGMA integrity_check;"

# Ver tabelas
sqlite3 "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db" ".tables"

# Ver preferências
sqlite3 "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db" "SELECT * FROM preferences;"
```

### Logs de Cache
```rust
// Adicionar logs para debug
eprintln!("[CACHE] Getting thumbnail for: {:?}", path);
eprintln!("[CACHE] Cache hit: {}", cache_hit);
eprintln!("[CACHE] Cache size: {} MB", size_bytes / 1024 / 1024);
```

## Performance Considerations

### Cache Size Limits
- **Padrão**: Sem limite hardcoded
- **Recomendação**: Monitorar crescimento
- **Cleanup**: Garbage collection automático a cada 3 segundos após startup

### I/O Optimization
```rust
// Writer/Reader separados para evitar lock contention
writer: Arc<Mutex<Connection>>, // Para writes
reader: Arc<Mutex<Connection>>,  // Para reads
```

### Index Optimization
```sql
-- Índices para queries comuns
CREATE INDEX idx_thumbnails_path ON thumbnails(file_path);
CREATE INDEX idx_thumbnails_modified ON thumbnails(modified_time);
CREATE INDEX idx_preferences_key ON preferences(key);
```

## Segurança e Privacidade

### Path Sanitization
```rust
fn sanitize_path(path: &Path) -> Result<PathBuf> {
    // Remove .. e outros elementos perigosos
    // Garante path está dentro do cache directory
}
```

### Permissions
- **Cache**: Acesso apenas ao usuário atual
- **Config**: Armazenado em LOCALAPPDATA (user-specific)
- **No dados sensíveis**: Apenas paths e metadados de arquivos

## Troubleshooting Comum

### "Database is locked"
```powershell
# Causa: Aplicativo ainda rodando ou crash anterior
# Solução: Finalizar processo e deletar .db-journal
Stop-Process -Name "mtt-file-manager" -Force
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.db-journal"
```

### "Failed to open database"
```powershell
# Causa: Database corrompido
# Solução: Deletar e recriar
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\thumbnails.db"
# O app recriará automaticamente
```

### Cache muito grande
```powershell
# Verificar tamanho
dir "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails" | Measure-Object -Property Length -Sum

# Limpar thumbnails antigos (mantém DB)
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager\thumbnails\*.webp"
```