# Build, Run e Debug - MTT File Manager

## Objetivo do Documento
Este documento descreve como compilar, executar e debugar o MTT File Manager, incluindo pré-requisitos, configurações e solução de problemas comuns.

## Pré-requisitos

### Rust Toolchain
```bash
# Instalar via rustup (recomendado no Linux/Mac)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Ou no Windows (PowerShell como Admin)
winget install Rustlang.Rustup
```

### MSVC Build Tools
- **Visual Studio Build Tools** ou **Visual Studio Community**
- Componentes necessários:
  - MSVC v143 - VS 2022 C++ x64/x86 build tools
  - Windows 10/11 SDK

### Dependências do Sistema
- **Windows 10** ou **Windows 11**
- **libmpv-2.dll** (para reprodução de vídeo)
- **Microsoft Edge WebView2 Runtime** (para visualização de PDFs)

### Instalação de Dependências Opcionais
```powershell
# Download libmpv (exemplo - ajustar versão conforme necessário)
# Baixar de: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Colocar libmpv-2.dll no mesmo diretório do executável ou no PATH

# WebView2 Runtime (geralmente já vem com Windows 11)
winget install Microsoft.EdgeWebView2Runtime
```

## Como Compilar

### Build de Desenvolvimento
```bash
# Clone o repositório
git clone <url-do-repositorio>
cd MTT-File-Manager-RUST

# Build debug do workspace completo (app + serviço de busca)
cargo build --workspace

# Build debug apenas do app
cargo build -p mtt-file-manager

# Executar em modo debug
cargo run
```

### Build de Produção
```bash
# Build release do workspace completo
cargo build --release --workspace

# Build release apenas do app
cargo build --release -p mtt-file-manager

# Build release apenas do serviço de busca
cargo build --release -p mtt-search-service

# Executar app
.\target\release\mtt-file-manager.exe

# Executar serviço em modo console (debug)
.\target\release\mtt-search-service.exe run-console
```

### Build com Features Específicas
```bash
# Build padrão (Drive Watcher + fallback notify-watcher)
cargo build

# Build sem features opcionais
# (sem fallback notify para UNC/rede; Drive Watcher local continua ativo)
cargo build --no-default-features
```

### Serviço de Busca Global
O serviço (`mtt-search-service`) roda como Windows Service e indexa todos os arquivos via USN Journal.

```powershell
# Instalar como serviço (requer PowerShell como Administrador)
.\target\release\mtt-search-service.exe install

# Iniciar o serviço
sc.exe start MTTFileManagerSearch

# Verificar status
sc.exe query MTTFileManagerSearch

# Parar o serviço
sc.exe stop MTTFileManagerSearch

# Remover o serviço
.\target\release\mtt-search-service.exe uninstall
```

## Flags e Features do Cargo

### Features Disponíveis
- **`notify-watcher`** - Habilita fallback via notify para paths UNC/rede
- **`default = ["notify-watcher"]`** - Feature padrão

**Nota**: O monitoramento principal usa Drive Watcher nativo (`ReadDirectoryChangesW` no drive raiz). A feature `notify-watcher` mantém o fallback para UNC/rede.

### Profiles de Build

#### Profile Dev (padrão)
```toml
[profile.dev]
opt-level = 0      # Sem otimizações
debug = true       # Inclui informações de debug
debug-assertions = true
overflow-checks = true
```

#### Profile Release (configurado no Cargo.toml)
```toml
[profile.release]
opt-level = 3      # Otimização máxima
lto = true         # Link Time Optimization
codegen-units = 1  # Compilação single-threaded (melhor otimização)
```

**Nota**: O profile release é otimizado para performance e tamanho de binário, mas aumenta significativamente o tempo de compilação.

## Como Executar com Logs

### Método 1: PowerShell Script (Recomendado)
```powershell
# Executar script que captura logs
.\run_with_logs.ps1

# Logs serão salvos em: debug_metadata_<timestamp>.log
```

### Método 2: Redirecionamento Manual
```powershell
# Executar com redirecionamento de stderr
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "app_debug.log"

# Ou simplesmente mostrar no console
.\target\release\mtt-file-manager.exe 2>&1

# Apenas erros
.\target\release\mtt-file-manager.exe 2> "errors.log"
```

### Método 3: Filtragem em Tempo Real
```powershell
# Com cores por categoria
.\target\release\mtt-file-manager.exe 2>&1 | ForEach-Object {
    if ($_ -match "ERROR") { Write-Host $_ -ForegroundColor Red }
    elseif ($_ -match "WARN") { Write-Host $_ -ForegroundColor Yellow }
    elseif ($_ -match "THUMB") { Write-Host $_ -ForegroundColor Cyan }
    else { Write-Host $_ -ForegroundColor Gray }
}

# Filtrar por categoria específica
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|PERF"
```

## Debug e Profiling

### Debug com VS Code
1. Instalar extensão "rust-analyzer"
2. Criar `.vscode/launch.json`:
```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "lldb",
            "request": "launch",
            "name": "Debug MTT File Manager",
            "cargo": {
                "args": ["build", "--bin", "mtt-file-manager"],
                "filter": {
                    "name": "mtt-file-manager",
                    "kind": "bin"
                }
            },
            "args": [],
            "cwd": "${workspaceFolder}"
        }
    ]
}
```

### Profiling com Flamegraph
```bash
# Instalar cargo-flamegraph
cargo install flamegraph

# Gerar flamegraph (requere admin no Windows)
cargo flamegraph --bin mtt-file-manager

# Resultado: flamegraph.svg
```

### Benchmarks
```bash
# Executar benchmarks
cargo bench

# Benchmark específico
cargo bench --bench shell_ops_blocking
```

### Verificar Dependências
```bash
# Árvore de dependências
cargo tree

# Verificar por vulnerabilidades
cargo install cargo-audit
cargo audit

# Verificar updates disponíveis
cargo install cargo-outdated
cargo outdated
```

## Solução de Problemas Comuns

### Erro: "libmpv-2.dll not found"
```powershell
# Solução: Copiar DLL para o diretório do executável
Copy-Item "caminho\para\libmpv-2.dll" -Destination ".\target\release\"

# Ou adicionar ao PATH
$env:PATH += ";C:\Path\To\libmpv"
```

### Erro: "WebView2 not available"
```powershell
# Instalar WebView2 Runtime
winget install Microsoft.EdgeWebView2Runtime

# Ou baixar manualmente: https://developer.microsoft.com/microsoft-edge/webview2/
```

### Build Lento
```bash
# Usar múltiplas threads para compilação
cargo build --release -j 8

# Ou setar permanentemente no .cargo/config.toml
[build]
jobs = 8
```

### Erro de Compilação: Windows API
```bash
# Verificar se Windows SDK está instalado
# Reinstalar componentes do Visual Studio:
# - MSVC v143
# - Windows 10/11 SDK
```

### Erro: "cannot find -lmpv"
```bash
# Verificar se libmpv está instalado
# No Windows, precisa da DLL no PATH ou no diretório do projeto
# mpv.lib deve estar no diretório raiz do projeto (já incluído)
```

## Comandos Úteis

### Build e Execução
```bash
# Desenvolvimento (workspace completo)
cargo build --workspace
cargo run

# Produção
cargo build --release --workspace

# App com logs
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "debug.log"

# Serviço em modo console
.\target\release\mtt-search-service.exe run-console
```

### Debug e Testes
```bash
# Executar benchmarks
cargo bench

# Verificar dependências
cargo tree
cargo audit

# Formatar código
cargo fmt

# Lint
cargo clippy

# Verificar (sem build)
cargo check
```

### Limpeza e Reset
```powershell
# Limpar cache da aplicação
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Limpar build
cargo clean

# Limpar tudo
cargo clean
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

## Variáveis de Ambiente

```powershell
# Backtrace em panics
$env:RUST_BACKTRACE=1           # ou "full" para completo

# Logging (se implementado com env_logger/tracing)
$env:RUST_LOG="debug"
$env:RUST_LOG="mtt_file_manager=debug"

# Configuração de desenvolvimento
$env:CARGO_INCREMENTAL=1        # Compilação incremental
```

## Configuração do Ambiente de Desenvolvimento

### .cargo/config.toml (opcional)
```toml
[build]
jobs = 8                        # Número de threads de compilação
rustflags = ["-C", "target-cpu=native"]

[target.x86_64-pc-windows-msvc]
linker = "rust-lld"             # Linker mais rápido (se disponível)
```

### VS Code Settings
```json
{
    "rust-analyzer.cargo.features": ["notify-watcher"],
    "rust-analyzer.checkOnSave.command": "clippy",
    "rust-analyzer.cargo.buildScripts.enable": true
}
```

## Dicas de Performance

### Build mais rápido
```bash
# Usar mold linker (Linux) ou lld (Windows)
# Instalar: cargo install -f cargo-binutils

# Compilação em paralelo
cargo build --release -j $(nproc)  # Linux
```

### Debug mais rápido
```bash
# Usar cargo check em vez de build
cargo check

# Verificar apenas pacote específico
cargo check -p mtt-file-manager
```

---

*Última atualização: 2026-02-11 (adicionado build do workspace e serviço de busca)*
