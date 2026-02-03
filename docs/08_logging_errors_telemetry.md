# Logs, Erros e Telemetria - MTT File Manager

## Objetivo do Documento
Este documento descreve como logs são gerados, erros são tratados e como capturar informações para suporte e debug.

## Sistema de Logs

### Abordagem Atual
O MTT File Manager usa **eprintln!()** para logging - logs vão para **stderr**.

### Categorias de Logs

| Categoria | Prefixo | Uso |
|-----------|---------|-----|
| **[INIT]** | Inicialização | Startup, carregamento de config |
| **[CACHE]** | Cache | Operações de cache (hit/miss) |
| **[THUMB]** | Thumbnails | Geração de thumbnails |
| **[THUMB_STAGE*]** | Thumbnail Stages | Estágios específicos (1-5) |
| **[FILE_OP]** | File Operations | Copiar, mover, deletar |
| **[NAV]** | Navigation | Navegação entre pastas |
| **[WORKER]** | Workers | Threads de background |
| **[ERROR]** | Erros | Erros críticos |
| **[WARN]** | Avisos | Warnings e fallbacks |
| **[PERF]** | Performance | Métricas de performance |
| **[WATCHER]** | Watcher | Eventos do filesystem watcher |
| **[PDF]** | PDF | Operações de PDF |
| **[MPV]** | MPV | Eventos do player de vídeo |
| **[FLOW]** | Fluxos | Debug de fluxos específicos |
| **[STATE]** | Estado | Dump de estado |

### Exemplos de Logs no Código
```rust
// Inicialização
eprintln!("[INIT] Starting application...");
eprintln!("[INIT] Cache directory: {:?}", cache_dir);

// Cache
eprintln!("[CACHE] Cache hit for: {:?}", path);
eprintln!("[CACHE] Cache miss, generating thumbnail...");

// Thumbnails
eprintln!("[THUMB] Requesting thumbnail for: {:?}", path);
eprintln!("[THUMB_STAGE2] Trying WIC for: {:?}", path);
eprintln!("[THUMB] All stages failed for: {:?}", path);

// Erros
eprintln!("[ERROR] Failed to load thumbnail: {}", error);
eprintln!("[WARN] Falling back to default icon");

// Performance
eprintln!("[PERF] Frame time: {:.2}ms", frame_ms);
eprintln!("[PERF] Rebuild took: {:.2}ms", elapsed_ms);
```

## Captura de Logs

### Método 1: PowerShell Script (Recomendado)
```powershell
# Executar script incluído no projeto
.\run_with_logs.ps1

# Logs serão salvos em: debug_metadata.log
```

**Conteúdo de `run_with_logs.ps1`**:
```powershell
$timestamp = Get-Date -Format "yyyy-MM-dd_HH-mm-ss"
$logFile = "debug_metadata_$timestamp.log"

Write-Host "Starting MTT File Manager with logging..."
Write-Host "Log file: $logFile"

.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath $logFile
```

### Método 2: Redirecionamento Manual
```powershell
# Capturar tudo (stdout + stderr)
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "app_debug.log"

# Apenas erros (stderr)
.\target\release\mtt-file-manager.exe 2> "errors.log"

# Filtrar por categoria
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "ERROR|WARN|THUMB" > "filtered.log"
```

### Método 3: Debug em Tempo Real
```powershell
# Ver logs em tempo real com cores
.\target\release\mtt-file-manager.exe 2>&1 | ForEach-Object {
    if ($_ -match "ERROR") { 
        Write-Host $_ -ForegroundColor Red 
    } elseif ($_ -match "WARN") { 
        Write-Host $_ -ForegroundColor Yellow 
    } elseif ($_ -match "THUMB") { 
        Write-Host $_ -ForegroundColor Cyan 
    } else { 
        Write-Host $_ -ForegroundColor Gray 
    }
}

# Com timestamp
.\target\release\mtt-file-manager.exe 2>&1 | 
    ForEach-Object { "[{0}] {1}" -f (Get-Date -Format "HH:mm:ss.fff"), $_ }
```

### Método 4: Filtragem Avançada
```powershell
# Apenas thumbnails
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "THUMB" | 
    Tee-Object "thumbnails.log"

# Performance + Erros
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "PERF|ERROR|frame_time" | 
    Tee-Object "perf_errors.log"

# Excluir linhas comuns (verbose)
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String -NotMatch "CACHE hit|Worker idle" | 
    Tee-Object "verbose.log"
```

## Sistema de Erros

### Tipo Principal: AppError
**Local**: `src/domain/errors.rs`

```rust
#[derive(Error, Debug)]
pub enum AppError {
    #[error("Security error: {0}")]
    Security(#[from] crate::infrastructure::security::SecurityError),
    
    #[error("Windows API error: {0}")]
    WindowsApi(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Thumbnail extraction failed for {path}: {source}")]
    ThumbnailExtraction {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    
    #[error("File operation failed: {0}")]
    FileOperation(String),
    
    #[error("Invalid state: {0}")]
    InvalidState(String),
    
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("Worker thread error: {0}")]
    Worker(String),
    
    #[error("UI rendering error: {0}")]
    UiRendering(String),
}

/// Tipo de resultado padrão
pub type AppResult<T> = Result<T, AppError>;
```

### Helpers de Erro
```rust
// Criar erros com contexto
pub fn windows_error(message: &str) -> AppError {
    AppError::WindowsApi(message.to_string())
}

pub fn file_operation_error(message: &str) -> AppError {
    AppError::FileOperation(message.to_string())
}

pub fn invalid_state_error(message: &str) -> AppError {
    AppError::InvalidState(message.to_string())
}

pub fn config_error(message: &str) -> AppError {
    AppError::Config(message.to_string())
}

pub fn worker_error(message: &str) -> AppError {
    AppError::Worker(message.to_string())
}

pub fn ui_rendering_error(message: &str) -> AppError {
    AppError::UiRendering(message.to_string())
}
```

### Macros de Erro
```rust
// safe_unwrap! - substitui .unwrap() com logging
let result = safe_unwrap!(operation, "context message");

// safe_expect! - substitui .expect() com contexto
let value = safe_expect!(option, "expected value to be present");

// ok_or_app_error - converte Option para Result
let value = option.ok_or_app_error("value not found")?;

// map_to_app_error - converte Result genérico
let value = result.map_to_app_error("operation failed")?;
```

## Stack Traces e Backtraces

### Habilitar Backtraces
```powershell
# Backtrace básico
$env:RUST_BACKTRACE=1
.\target\release\mtt-file-manager.exe

# Backtrace completo
$env:RUST_BACKTRACE=full
.\target\release\mtt-file-manager.exe

# Com logging
$env:RUST_BACKTRACE=1
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "crash.log"
```

### Capturar Panics
O sistema atualmente usa o panic handler padrão do Rust. Para debugging:

```rust
// Adicionar no main.rs para capturar panics
std::panic::set_hook(Box::new(|panic_info| {
    eprintln!("[PANIC] Application panicked: {}", panic_info);
    if let Some(location) = panic_info.location() {
        eprintln!("[PANIC] Location: {}:{}", location.file(), location.line());
    }
    // Opcional: salvar em arquivo
    std::fs::write("panic.log", format!("{:?}", panic_info));
}));
```

## Telemetria e Métricas

### Métricas de Performance
**Local**: `src/ui/app_impl.rs`

```rust
// Frame timing
let frame_ms = (ctx.input(|i| i.stable_dt) * 1000.0) as f32;
self.frame_time_avg_ms = self.frame_time_avg_ms * 0.9 + frame_ms * 0.1;
self.fps_avg = if self.frame_time_avg_ms > 0.0 {
    1000.0 / self.frame_time_avg_ms
} else {
    0.0
};

// Log periódico
if self.frame_count % 60 == 0 {
    eprintln!("[PERF] FPS: {:.1}, Frame: {:.2}ms", 
        self.fps_avg, self.frame_time_avg_ms);
}
```

### Cache Metrics
```rust
// Cache hit rate (exemplo)
let hits = cache.hits.load(Ordering::Relaxed);
let misses = cache.misses.load(Ordering::Relaxed);
let total = hits + misses;
if total > 0 {
    let hit_rate = hits as f64 / total as f64;
    eprintln!("[CACHE] Hit rate: {:.1}%", hit_rate * 100.0);
}
eprintln!("[CACHE] Entries: {}", cache.len());
```

### Memory Usage (Externo)
```powershell
# Monitorar uso de memória
while ($true) {
    $proc = Get-Process mtt-file-manager -ErrorAction SilentlyContinue
    if ($proc) {
        $mb = [math]::Round($proc.WorkingSet / 1MB, 2)
        Write-Host "$(Get-Date -Format 'HH:mm:ss') - Memory: $mb MB"
    }
    Start-Sleep -Seconds 5
}
```

## Debugging de Problemas Específicos

### 1. Thumbnails Não Geram
```powershell
# Filtrar logs de thumbnail
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "THUMB|ERROR" | 
    Tee-Object "thumb_debug.log"

# Verificar stages específicos
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "THUMB_STAGE|failed"
```

### 2. Performance Issues
```powershell
# Ver métricas de frame time
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "frame_time|fps|PERF" | 
    Tee-Object "perf.log"

# Ver rebuild de items
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "Rebuild|filter|sort"
```

### 3. Problemas de Navegação
```powershell
# Logs de navegação
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "NAV|LOAD|folder" | 
    Tee-Object "nav.log"
```

### 4. Problemas de Workers
```powershell
# Logs de workers
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "WORKER|channel|sender|receiver"
```

### 5. Problemas de Vídeo/PDF
```powershell
# Logs de MPV
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "MPV|video"

# Logs de PDF
.\target\release\mtt-file-manager.exe 2>&1 | 
    Select-String "PDF|WebView"
```

## Variáveis de Ambiente Úteis

```powershell
# Backtrace em panics
$env:RUST_BACKTRACE=1           # ou "full"

# Logging (se implementado com env_logger)
$env:RUST_LOG="debug"
$env:RUST_LOG="mtt_file_manager=debug"

# Diretório de cache customizado (não implementado)
# $env:MTT_CACHE_DIR="D:\\Custom\\Cache"
```

## Scripts de Diagnóstico

### Script Completo de Diagnóstico
```powershell
# save_diagnostic_info.ps1
$timestamp = Get-Date -Format "yyyy-MM-dd_HH-mm-ss"
$diagDir = "MTT-Diagnostics-$timestamp"
New-Item -ItemType Directory -Path $diagDir

# Info do sistema
systeminfo | Out-File "$diagDir\system_info.txt"
Get-ComputerInfo | Out-File "$diagDir\computer_info.txt"

# Processos
Get-Process | Where-Object {$_.Name -like "*mtt*"} | 
    Out-File "$diagDir\processes.txt"

# Variáveis de ambiente relevantes
@"
RUST_BACKTRACE: $env:RUST_BACKTRACE
RUST_LOG: $env:RUST_LOG
LOCALAPPDATA: $env:LOCALAPPDATA
"@ | Out-File "$diagDir\env_vars.txt"

# Cache
$cacheDir = "$env:LOCALAPPDATA\MTT-File-Manager"
if (Test-Path $cacheDir) {
    Get-ChildItem $cacheDir -Recurse | 
        Out-File "$diagDir\cache_structure.txt"
}

# Executar com logs
Write-Host "Running MTT File Manager with logging..."
.\target\release\mtt-file-manager.exe 2>&1 | 
    Tee-Object "$diagDir\app.log"

Write-Host "Diagnostics saved to: $diagDir"
```

---

*Última atualização: 2026-02-03 (pós-refatoração)*
