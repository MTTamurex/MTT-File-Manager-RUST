# Logs, Erros e Telemetria - MTT File Manager

## Objetivo do Documento
Este documento descreve como logs são gerados, erros são tratados e como capturar informações para suporte e debug.

## Sistema de Logs

### Abordagem Atual
O MTT File Manager usa **eprintln!()** para logging - logs vão para **stderr**.

### Locais de Log
```rust
// Exemplos de logs no código
eprintln!("[INIT] Initializing app...");
eprintln!("[CACHE] Cache hit for: {:?}", path);
eprintln!("[ERROR] Failed to load thumbnail: {}", error);
eprintln!("[WARN] Falling back to default icon");
```

### Categorias de Logs
- **[INIT]** - Inicialização e startup
- **[CACHE]** - Operações de cache
- **[THUMB]** - Geração de thumbnails
- **[ERROR]** - Erros críticos
- **[WARN]** - Avisos e fallbacks
- **[PERF]** - Performance metrics
- **[WATCHER]** - File system watcher events

## Captura de Logs

### Método 1: PowerShell Script
```powershell
# Executar script incluído
.\run_with_logs.ps1

# Saída: debug_metadata.log
```

### Método 2: Redirecionamento Manual
```powershell
# Capturar tudo
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "app.log"

# Apenas erros
.\target\release\mtt-file-manager.exe 2> "errors.log"

# Filtrar por categoria
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "ERROR|WARN" > "errors_only.log"
```

### Método 3: Debug em Tempo Real
```powershell
# Ver logs em tempo real
.\target\release\mtt-file-manager.exe 2>&1 | ForEach-Object { Write-Host $_ -ForegroundColor Yellow }

# Com cores para diferentes níveis
.\target\release\mtt-file-manager.exe 2>&1 | ForEach-Object {
    if ($_ -match "ERROR") { Write-Host $_ -ForegroundColor Red }
    elseif ($_ -match "WARN") { Write-Host $_ -ForegroundColor Yellow }
    else { Write-Host $_ -ForegroundColor Gray }
}
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
```

### Macros de Erro
```rust
// Substituir unwrap() com logging
let result = safe_unwrap!(operation, "Failed to load file");

// Substituir expect() com contexto rico
let value = safe_expect!(option, "Expected value to be present");

// Converter Option para Result com contexto
let value = option.ok_or_app_error("Value not found")?;
```

## Stack Traces e Backtraces

### Habilitar Backtraces
```powershell
# Setar variável de ambiente
$env:RUST_BACKTRACE=1
.\target\release\mtt-file-manager.exe

# Backtrace completo
$env:RUST_BACKTRACE=full
.\target\release\mtt-file-manager.exe
```

### Capturar Panics
```rust
// Exemplo de panic handler (se implementado)
std::panic::set_hook(Box::new(|panic_info| {
    eprintln!("[PANIC] Application panicked: {}", panic_info);
    if let Some(location) = panic_info.location() {
        eprintln!("[PANIC] Location: {}:{}", location.file(), location.line());
    }
}));
```

## Telemetria e Métricas

### Métricas de Performance
```rust
// Frame timing (ui/app_impl.rs)
let frame_ms = (ctx.input(|i| i.stable_dt) * 1000.0) as f32;
self.frame_time_avg_ms = self.frame_time_avg_ms * 0.9 + frame_ms * 0.1;
self.fps_avg = if self.frame_time_avg_ms > 0.0 {
    1000.0 / self.frame_time_avg_ms
} else {
    0.0
};
```

### Cache Metrics
```rust
// Cache hit/miss tracking
eprintln!("[CACHE] Hit rate: {:.1}%", cache.hit_rate() * 100.0);
eprintln!("[CACHE] Total entries: {}", cache.len());
```

### Memory Usage
```powershell
# Monitorar uso de memória externamente
Get-Process mtt-file-manager | Select-Object WorkingSet, PeakWorkingSet
```

## Debugging de Problemas Específicos

### 1. Thumbnails Não Geram
```powershell
# Filtrar logs de thumbnail
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|ERROR" | Tee-Object "thumb_debug.log"

# Verificar com debugger
eprintln!("[THUMB] Requesting: {:?}", path);
eprintln!("[THUMB] Worker queue size: {}", queue.len());
eprintln!("[THUMB] Cache hit: {}", cache.contains(path));
```

### 2. Performance Issues
```powershell
# Ver métricas de frame time
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "frame_time|fps" | Tee-Object "perf.log"

# Análise simples
Get-Content perf.log | ForEach-Object {
    if ($_ -match "frame_time_avg_ms: ([\d.]+)") {
        $frameTime = [double]$matches[1]
        if ($frameTime > 16.67) { # > 60 FPS
            Write-Host "Performance issue: ${frameTime}ms" -ForegroundColor Red
        }
    }
}
```

### 3. File Operations Fail
```powershell
# Filtrar operações de arquivo
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "FILE_OP|ERROR" | Tee-Object "fileops.log"

# Verificar erros específicos
Get-Content fileops.log | Select-String "Access is denied|Path not found"
```

### 4. Watcher Issues
```powershell
# Monitorar watcher events
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "WATCHER" | Tee-Object "watcher.log"

# Verificar frequência de eventos
Get-Content watcher.log | Measure-Object -Line
```

## Reportando Issues

### Informações Necessárias
1. **Logs completos**: Arquivo .log gerado com métodos acima
2. **Sistema**: Windows versão, arquitetura
3. **Reprodução**: Passos exatos para reproduzir
4. **Contexto**: O que estava fazendo quando ocorreu
5. **Arquivos**: Tipo de arquivo, tamanho, localização

### Template de Bug Report
```markdown
**Descrição**: 

**Para Reproduzir**:
1. Navegar para pasta '...'
2. Clicar em '...'
3. Ver erro '...'

**Comportamento Esperado**:

**Logs**: [anexar arquivo .log]

**Sistema**:
- Windows: [versão]
- Versão do app: [git commit ou release]
- Hardware: [CPU, RAM, GPU]

**Arquivos Envolvidos**:
- Tipo: [imagem, vídeo, pasta]
- Tamanho: [MB/GB]
- Localização: [local, rede]
```

## Performance Profiling

### Benchmarks Incluídos
```bash
# Executar benchmarks
cargo bench --bench shell_ops_blocking

# Resultados em target/criterion/
```

### Profiling Manual
```powershell
# Medir startup time
Measure-Command { .\target\release\mtt-file-manager.exe }

# Monitorar durante uso
Get-Process mtt-file-manager | Format-Table -Property 
    Name, CPU, WorkingSet, VirtualMemorySize, 
    StartTime, TotalProcessorTime -AutoSize
```

## Logs de Sistema Windows (Event Viewer)

### Application Crashes
```powershell
# Ver eventos de crash
Get-EventLog -LogName Application -Source "Application Error" | 
    Where-Object { $_.Message -match "mtt-file-manager" } |
    Format-List TimeGenerated, Message
```

### Windows Error Reporting
```powershell
# Ver relatórios de erro
Get-ChildItem "C:\ProgramData\Microsoft\Windows\WER\ReportQueue\" |
    Where-Object { $_.Name -match "mtt-file-manager" }
```

## Debugging Avançado

### Debug Build com Símbolos
```bash
# Build com debug symbols
cargo build

# Executar com debugger
# VS Code: F5 com configuração apropriada
# WinDbg: Abrir .exe com PDBs
```

### Memory Debugging
```rust
// Adicionar logs de memória (se necessário)
eprintln!("[MEM] Texture cache size: {} bytes", cache_size);
eprintln!("[MEM] Pending thumbnails: {}", pending_count);
```

### Thread Debugging
```rust
// Verificar estado de threads
eprintln!("[THREAD] Active workers: {}", active_workers);
eprintln!("[THREAD] Queue size: {}", queue.len());
```

## Checklist para Suporte

### Antes de Reportar
- [ ] Logs capturados com método apropriado
- [ ] Tentativa de reprodução com passos claros
- [ ] Sistema e versão documentados
- [ ] Arquivo de configuração não corrompido
- [ ] Dependências externas instaladas (libmpv, WebView2)

### Informações a Incluir
- [ ] Arquivo .log completo (anexar, não colar)
- [ ] Versão do Windows (winver)
- [ ] Commit SHA ou versão do release
- [ ] Hardware relevante (especialmente GPU)
- [ ] Tipo e tamanho dos arquivos envolvidos
- [ ] Logs do Event Viewer se houver crash

### Comandos de Diagnóstico
```powershell
# System info
systeminfo | findstr /B /C:"OS Name" /C:"OS Version"

# GPU info
Get-WmiObject win32_VideoController | Format-List Name, DriverVersion

# Disk space
Get-WmiObject win32_logicaldisk | Format-Table DeviceID, @{Name="Size(GB)";Expression={[math]::Round($_.Size/1GB,2)}}, @{Name="Free(GB)";Expression={[math]::Round($_.FreeSpace/1GB,2)}}

# Memory
Get-WmiObject win32_physicalmemory | Measure-Object -Property capacity -Sum | %{[math]::Round($_.sum/1GB,2)}
```