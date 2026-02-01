# Stack de Dependências - MTT File Manager

## Objetivo do Documento
Este documento detalha todas as dependências do projeto, suas versões, propósitos e features utilizadas.

## Dependências Principais

### GUI Framework
```toml
eframe = { version = "0.31", features = ["persistence"] }
```
- **Propósito**: Framework de GUI multiplataforma baseado em egui
- **Features usadas**: `persistence` - para salvar estado da janela
- **Alternativas consideradas**: iced, druid
- **Notas**: Immediate mode GUI, ideal para aplicações desktop

### Processamento Paralelo
```toml
rayon = "1.10"
```
- **Propósito**: Paralelização de dados (data parallelism)
- **Uso**: Processamento de thumbnails, leitura de diretórios
- **Exemplo**: `par_iter()` em coleções de arquivos

### File System
```toml
walkdir = "2.5"
```
- **Propósito**: Iteração recursiva sobre diretórios
- **Uso**: Cálculo de tamanho de pastas, varredura

```toml
notify = { version = "6.1.1", optional = true }
```
- **Propósito**: Monitoramento de mudanças no filesystem
- **Feature**: `notify-watcher` (default)
- **Nota**: Usa ReadDirectoryChangesW no Windows (não requer admin)

### Cache e Performance
```toml
lru = "0.12"
```
- **Propósito**: Cache LRU em memória
- **Uso**: Cache de metadados, thumbnails na memória

```toml
dashmap = "5.5"
```
- **Propósito**: HashMap concorrente
- **Uso**: Cache thread-safe de texturas

```toml
rustc-hash = "2.0"
```
- **Propósito**: Hash mais rápida para PathBuf
- **Uso**: FxHashSet/FxHashMap para chaves de path

```toml
fxhash = "0.2.1"
```
- **Propósito**: Hash rápida (alternativa ao rustc-hash)
- **Uso**: Conjuntos de paths em memória

### Processamento de Imagem
```toml
image = { version = "0.25", features = ["webp", "gif"] }
```
- **Propósito**: Decodificação/encodificação de imagens
- **Features**: Suporte a WebP e GIF
- **Uso**: Geração de thumbnails, processamento de imagens

```toml
webp = "0.3"
```
- **Propósito**: Compressão WebP com controle de qualidade
- **Uso**: Thumbnails com compressão lossy

```toml
kamadak-exif = "0.5"
```
- **Propósito**: Leitura direta de EXIF de JPEGs
- **Uso**: Metadados de imagens fotográficas

### SVG e Vetores
```toml
resvg = "0.44"
```
- **Propósito**: Renderização de SVG
- **Uso**: Ícones vetoriais, thumbnails de SVG

```toml
usvg = "0.44"
```
- **Propósito**: Parsing de SVG
- **Uso**: Preparação para renderização

```toml
tiny-skia = "0.11"
```
- **Propósito**: Backend de rasterização para resvg
- **Uso**: Conversão SVG → bitmap

### Banco de Dados
```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```
- **Propósito**: SQLite embutido
- **Feature**: `bundled` - SQLite estático
- **Uso**: Cache persistente de thumbnails

### Vídeo e Mídia
```toml
mpv = { package = "libmpv2", version = "5.0.3" }
```
- **Propósito**: Bindings para libmpv
- **Uso**: Reprodução de vídeo
- **Runtime**: Requer `libmpv-2.dll`

### Serialização
```toml
serde_json = "1.0"
```
- **Propósito**: JSON parsing/serialization
- **Uso**: Configurações, metadados

### Comunicação entre Threads
```toml
crossbeam-channel = "0.5.15"
```
- **Propósito**: Canais MPSC de alta performance
- **Uso**: Comunicação UI ↔ Workers

### Diretórios e Paths
```toml
dirs = "5.0"
```
- **Propósito**: Diretórios padrão do sistema
- **Uso**: Cache em `%LOCALAPPDATA%\MTT-File-Manager`

### Área de Transferência
```toml
clipboard-win = "5.4"
```
- **Propósito**: Integração com clipboard Windows
- **Formato**: CF_HDROP para arquivos
- **Uso**: Copiar/colar arquivos

### Diálogos de Arquivo
```toml
rfd = "0.15"
```
- **Propósito**: File dialogs multiplataforma
- **Uso**: Diálogos de seleção de arquivo/pasta

### Ordenação Natural
```toml
natord = "1.0"
```
- **Propósito**: Ordenação natural de strings
- **Uso**: Ordenação de nomes de arquivo (File1, File2, File10)

### Temporários
```toml
tempfile = "3.10"
```
- **Propósito**: Arquivos temporários seguros
- **Uso**: Operações de arquivo, cache temporário

### Tratamento de Erros
```toml
thiserror = "2.0"
```
- **Propósito**: Derivação de tipos de erro
- **Uso**: `AppError` e tipos de erro customizados

### Raw Window Handle
```toml
raw-window-handle = "0.6"
```
- **Propósito**: Acesso raw a handles de janela
- **Uso**: Integração com mpv (necessário para embedding)

## Windows API Dependencies

```toml
[dependencies.windows]
version = "0.61.0"
features = [
    "Win32_UI_Shell",                    # Shell API (explorer integration)
    "Win32_UI_Shell_Common",             # Tipos comuns do shell
    "Win32_UI_Shell_PropertiesSystem",  # Propriedades de arquivos
    "Win32_System_Com",                  # COM (Component Object Model)
    "Win32_System_Com_StructuredStorage", # Storage estruturado
    "Win32_System_DataExchange",         # Clipboard, etc
    "Win32_System_Memory",               # Gerenciamento de memória
    "Win32_System_Registry",             # Registry access
    "Win32_Graphics_Gdi",                # GDI (Graphics Device Interface)
    "Win32_Foundation",                   # Tipos básicos
    "Win32_Storage_FileSystem",          # File system APIs
    "Win32_UI_WindowsAndMessaging",     # Janelas e mensagens
    "Win32_System_ProcessStatus",         # Informações de processo
    "Win32_System_Threading",           # Threads
    "Win32_Graphics_Imaging",             # WIC (Windows Imaging Component)
    "Win32_Graphics_Dwm",               # Desktop Window Manager
    "Win32_Media_MediaFoundation",      # Media Foundation
    "Win32_Devices_DeviceAndDriverInstallation", # Devices
    "Win32_System_LibraryLoader",        # Carregamento de DLLs
    "Win32_System_Ioctl",                # I/O Control
    "Win32_UI_Input_KeyboardAndMouse",  # Input
    "Win32_System_Variant",              # Variants COM
    "Win32_System_Search_Common",       # Search APIs
    "Win32_Storage_Vhd",                # Virtual Hard Disks
    "Win32_Security",                    # Segurança
    "Win32_System_IO",                  # I/O operations
    "Win32_System_WindowsProgramming",  # Programação Windows
]
```

## Build Dependencies

```toml
[build-dependencies]
winresource = "0.1"
```
- **Propósito**: Embed de recursos Windows (ícones, versão)
- **Uso**: `build.rs` - Adiciona ícone ao executável

## Dev Dependencies

```toml
[dev-dependencies]
criterion = "0.5"
```
- **Propósito**: Framework de benchmarking
- **Uso**: Benchmarks de performance (ex: `benches/shell_ops_blocking.rs`)

## Features do Cargo

```toml
[features]
default = ["notify-watcher"]
notify-watcher = ["notify"]
```

### Feature: `notify-watcher` (padrão)
- **Ativa**: `notify` crate
- **Uso**: Monitoramento cross-platform de filesystem
- **Implementação Windows**: Usa `ReadDirectoryChangesW` API
- **Nota**: Não requer privilégios de administrador

## Profiles de Build

```toml
[profile.release]
opt-level = 3      # Otimização máxima
lto = true         # Link Time Optimization
codegen-units = 1  # Compilação single-thread (melhor otimização)
```

### Impacto no Binário
- **Tamanho**: ~15-20MB (release)
- **Memória**: ~50-100MB em uso
- **Startup**: <1s em SSD
- **Performance**: 60 FPS estável

## Integrações Externas Necessárias

### Runtime Dependencies
1. **libmpv-2.dll**
   - Download: https://sourceforge.net/projects/mpv-player-windows/
   - Local: Mesmo diretório do executável ou PATH

2. **Microsoft Edge WebView2 Runtime**
   - Download: https://developer.microsoft.com/microsoft-edge/webview2/
   - Instalação: Winglet ou manual
   - Uso: Visualização de PDFs

### Fontes do Sistema
- **Segoe UI**: Fonte principal (Windows)
- **Segoe UI Symbol**: Símbolos
- **Arial Unicode**: Fallback Unicode (opcional, 22MB)
- **Remix Icon**: Fonte de ícones (embarcada)

## Alternativas Consideradas

| Funcionalidade | Escolhida | Alternativas | Razão da Escolha |
|----------------|-----------|--------------|------------------|
| GUI | eframe/egui | iced, druid | Immediate mode, performance |
| Vídeo | libmpv | ffmpeg-next | Simplicidade, performance |
| PDF | WebView2 | pdfium, poppler | Integração Windows |
| Cache | SQLite | sled, rocksdb | Confiabilidade, tooling |
| Windows API | windows-rs | winapi | Bindings seguros, ativo |

## Compatibilidade

### Sistemas Operacionais
- **Windows 10**: ✅ Suportado
- **Windows 11**: ✅ Suportado
- **Windows 7/8**: ❌ Não testado (APIs modernas)
- **Linux/macOS**: ❌ Não suportado (Windows APIs)

### Arquiteturas
- **x86_64**: ✅ Suportado
- **ARM64**: ❌ Não testado
- **x86**: ❌ Não suportado

### Filesystems
- **NTFS**: ✅ Suportado
- **FAT32**: ✅ Suportado
- **exFAT**: ✅ Suportado
- **ReFS**: ⚠️ Parcial (não testado)

**Nota**: O monitoramento de mudanças usa `notify` crate que funciona em qualquer filesystem suportado pelo Windows, sem requerer USN Journal.

## Notas de Segurança

### Verificações Implementadas
- **Path traversal**: Prevenido via `std::path` validation
- **Symbolic links**: Seguidos com cuidado
- **File permissions**: Respeitados via Windows APIs
- **COM security**: Inicialização apropriada

### Dependências Vulneráveis
- **Nenhuma conhecida**: Todas as deps atualizadas
- **Auditoria**: `cargo audit` limpo
- **Licenças**: Compatíveis (MIT/Apache-2.0 predominantemente)