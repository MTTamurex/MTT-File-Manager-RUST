# Build, Run e Debug - MTT File Manager

## Objetivo do Documento
Este documento descreve como compilar, executar e debugar o MTT File Manager, incluindo pré-requisitos, configurações e solução de problemas comuns.

## Pré-requisitos

### Rust Toolchain
```bash
# Instalar via rustup (recomendado)
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

# Build debug (mais rápido, sem otimizações)
cargo build

# Executar em modo debug
cargo run
```

### Build de Produção
```bash
# Build release (otimizado, maior tempo de compilação)
cargo build --release

# Executar release
cargo run --release

# Ou executar diretamente
.\target\release\mtt-file-manager.exe
```

### Build com Features Específicas
```bash
# Build padrão (com notify-watcher)
cargo build

# Build sem features opcionais
cargo build --no-default-features
```

## Flags e Features do Cargo

### Features Disponíveis
- **`notify-watcher`** - Usa notify crate para watcher de filesystem (cross-platform)
- **`default = ["notify-watcher"]`** - Feature padrão

**Nota**: O monitoramento usa `notify` crate que implementa `ReadDirectoryChangesW` no Windows. Não requer privilégios de administrador.

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

## Como Executar com Logs

### Método 1: PowerShell Script
```powershell
# Executar script que captura logs
.\run_with_logs.ps1

# Logs serão salvos em: debug_metadata.log
```

### Método 2: Redirecionamento Manual
```powershell
# Executar com redirecionamento de stderr
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object -FilePath "app_debug.log"

# Ou simplesmente mostrar no console
.\target\release\mtt-file-manager.exe 2>&1
```

### Método 3: Variáveis de Ambiente
```powershell
# Setar variável de debug (se implementado)
$env:MTT_DEBUG="1"
$env:RUST_LOG="debug"
cargo run
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

### Profiling com Perf
```bash
# Instalar cargo-flamegraph
cargo install flamegraph

# Gerar flamegraph
cargo flamegraph --bin mtt-file-manager

# Resultado: flamegraph.svg
```

### Benchmarks
```bash
# Executar benchmarks (se houver)
cargo bench

# Benchmark específico
cargo bench --bench shell_ops_blocking
```

## Solução de Problemas Comuns

### Erro: "libmpv-2.dll not found"
```powershell
# Solução: Copiar DLL para o diretório do executável
Copy-Item "caminho\para\libmpv-2.dll" -Destination ".\target\release\"
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

### Erro de Compilação: "Windows API not found"
```powershell
# Verificar se Windows SDK está instalado
# Reinstalar via Visual Studio Installer
# Certificar-se de que está usando toolchain MSVC
rustup default stable-msvc
```

### Runtime Error: "Failed to create window"
```powershell
# Verificar drivers de vídeo
# Atualizar DirectX
# Executar como administrador (para testar)
```

### Problemas de Performance
```powershell
# Verificar uso de CPU/memória
Get-Process mtt-file-manager | Select-Object CPU, WorkingSet

# Logs de performance estão em: hd_perf_check.txt
```

## Configurações de Desenvolvimento

### Cargo Config
Arquivo `.cargo/config.toml`:
```toml
[build]
target = "x86_64-pc-windows-msvc"

[env]
RUST_LOG = "debug"
MTT_DEBUG = "1"
```

### Variáveis de Ambiente Úteis
```powershell
# Debug logging
$env:RUST_LOG="debug"
$env:RUST_BACKTRACE=1

# Performance profiling
$env:CARGO_PROFILE_RELEASE_DEBUG=true
```

## Testes

### Executar Testes
```bash
# Todos os testes
cargo test

# Testes específicos
cargo test --lib
cargo test --bin mtt-file-manager

# Testes com output
cargo test -- --nocapture
```

### Testes de Infrastructure
```bash
# Testes de hardware (se houver)
cargo test --test infrastructure
```

## Empacotamento e Distribuição

### Criar Installer (se houver script)
```powershell
# Verificar se há script de packaging
# Isso seria implementado externamente
```

### Arquivos Necessários para Distribuição
```
mtt-file-manager.exe
libmpv-2.dll (se não estiver no PATH)
virtual_drive_config.json (configurações padrão)
README.md
LICENSE (se existir)
```

## Onde os Outputs Ficam

### Builds
- **Debug**: `.\target\debug\mtt-file-manager.exe`
- **Release**: `.\target\release\mtt-file-manager.exe`

### Logs e Debug
- **Logs da aplicação**: Console stderr (redirecionável)
- **Logs do script**: `debug_metadata.log`
- **Performance**: `hd_perf_check.txt`
- **Cache**: `%LOCALAPPDATA%\MTT-File-Manager\`

### Cache e Configurações
- **Thumbnails**: `%LOCALAPPDATA%\MTT-File-Manager\thumbnails\`
- **Configurações**: SQLite em `thumbnails.db`
- **Logs WebView2**: `.\target\release\mtt-file-manager.exe.WebView2\`