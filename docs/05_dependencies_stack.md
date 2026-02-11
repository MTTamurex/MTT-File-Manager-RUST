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
- **Uso**: Processamento de thumbnails, leitura de diretórios, ordenação
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
- **Feature**: `notify-watcher` (default, fallback para UNC/rede)
- **Nota**: Monitoramento principal é feito por Drive Watcher nativo; `notify` cobre fallback

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
- **Uso**: Cache thread-safe de texturas (TextureCache)

```toml
rustc-hash = "2.0"
```
- **Propósito**: Hash mais rápida para PathBuf (FxHashSet/FxHashMap)
- **Uso**: Sets e maps com chaves de path

```toml
fxhash = "0.2.1"
```
- **Propósito**: Hash rápida (alternativa)
- **Uso**: Conjuntos de paths em memória

### Processamento de Imagem
```toml
image = { version = "0.25", features = ["webp", "gif"] }
```
- **Propósito**: Decodificação/encodificação de imagens
- **Features**: Suporte a WebP e GIF animado
- **Uso**: Geração de thumbnails, processamento de imagens (Stage 1)

```toml
webp = "0.3"
```
- **Propósito**: Compressão WebP com controle de qualidade
- **Uso**: Thumbnails com compressão lossy para cache em disco

```toml
kamadak-exif = "0.5"
```
- **Propósito**: Leitura direta de EXIF de JPEGs
- **Uso**: Metadados de imagens fotográficas (orientação, data, câmera)

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
- **Feature**: `bundled` - SQLite estático (sem dependência externa)
- **Uso**: Cache persistente de thumbnails e preferências (app), índice de arquivos (serviço de busca)
- **Nota**: Workspace dependency compartilhada (usado por `mtt-file-manager` e `mtt-search-service`)

### Vídeo e Mídia
```toml
mpv = { package = "libmpv2", version = "5.0.3" }
```
- **Propósito**: Bindings para libmpv
- **Uso**: Reprodução de vídeo
- **Runtime**: Requer `libmpv-2.dll` no PATH ou diretório do executável

```toml
raw-window-handle = "0.6"
```
- **Propósito**: Acesso raw a handles de janela
- **Uso**: Integração com mpv (necessário para embedding do player)

### Serialização e IPC
```toml
serde_json = "1.0"
```
- **Propósito**: JSON parsing/serialization
- **Uso**: Configurações, metadados, virtual_drive_config.json

```toml
serde = { version = "1.0", features = ["derive"] }
```
- **Propósito**: Framework de serialização/deserialização
- **Uso**: Protocolo IPC (bincode), configurações
- **Nota**: Workspace dependency compartilhada entre os 3 crates

```toml
bincode = "1.3"
```
- **Propósito**: Serialização binária compacta e rápida
- **Uso**: Protocolo IPC entre app e serviço de busca (Named Pipes)
- **Nota**: Workspace dependency compartilhada

### Comunicação entre Threads
```toml
crossbeam-channel = "0.5.15"
```
- **Propósito**: Canais MPSC de alta performance
- **Uso**: Comunicação UI ↔ Workers (thumbnails, ícones, metadados, operações)

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
- **Uso**: Copiar/colar arquivos nativamente

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
- **Uso**: Ordenação de nomes de arquivo (File1, File2, File10 em vez de File1, File10, File2)

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
- **Uso**: `AppError` e tipos de erro customizados com `#[derive(Error)]`

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
    "Win32_Foundation",                   # Tipos básicos (HANDLE, HWND, etc)
    "Win32_Storage_FileSystem",          # File system APIs
    "Win32_UI_WindowsAndMessaging",     # Janelas e mensagens
    "Win32_System_ProcessStatus",        # Informações de processo
    "Win32_System_Threading",            # Threads
    "Win32_Graphics_Imaging",            # WIC (Windows Imaging Component)
    "Win32_Graphics_Dwm",                # Desktop Window Manager
    "Win32_Media_MediaFoundation",       # Media Foundation
    "Win32_Devices_DeviceAndDriverInstallation", # Dispositivos
    "Win32_System_LibraryLoader",        # Carregamento de DLLs
    "Win32_System_Ioctl",                # I/O control
    "Win32_UI_Input_KeyboardAndMouse",   # Input
    "Win32_System_Variant",              # VARIANT para COM
    "Win32_System_Search_Common",        # Search
    "Win32_Storage_Vhd",                 # Virtual Hard Disk (ISO)
    "Win32_Security",                    # Segurança
    "Win32_System_IO",                   # I/O APIs
    "Win32_System_Pipes",               # Named Pipes (IPC com serviço de busca)
    "Win32_System_WindowsProgramming",   # APIs gerais do Windows
]
```

### Features Windows Detalhadas

| Feature | Propósito |
|---------|-----------|
| `Win32_UI_Shell` | Integração com Explorer (IShellItem, IFileOperation) |
| `Win32_UI_Shell_PropertiesSystem` | Propriedades de arquivos (metadados) |
| `Win32_System_Com` | COM para APIs Windows |
| `Win32_Graphics_Gdi` | Bitmaps, device contexts |
| `Win32_Graphics_Imaging` | WIC para thumbnails |
| `Win32_Media_MediaFoundation` | Extração de frames de vídeo |
| `Win32_Storage_FileSystem` | Operações de arquivo nativas |
| `Win32_UI_WindowsAndMessaging` | Janelas, mensagens, subclassing |
| `Win32_Storage_Vhd` | Montagem de ISOs |
| `Win32_System_Pipes` | Named Pipes para IPC com serviço de busca |

## Features do Cargo

```toml
[features]
default = ["notify-watcher"]
notify-watcher = ["notify"]
```

### `notify-watcher` (Default)
- Habilita fallback de monitoramento para UNC/rede via `notify` crate
- Complementa o Drive Watcher nativo usado em drives locais
- Não requer privilégios de administrador
- Pode ser desabilitado com: `cargo build --no-default-features` (desativa apenas o fallback notify)

## Dependências do Serviço de Busca (mtt-search-service)

O serviço de busca é um binário separado com suas próprias dependências:

### Windows Service
```toml
windows-service = "0.7"
```
- **Propósito**: Integração com Windows Service Control Manager (SCM)
- **Uso**: Registro, controle de ciclo de vida e dispatch do serviço

### Windows API (serviço)
```toml
[dependencies.windows]
version = "0.61.0"
features = [
    "Win32_Foundation",          # Tipos básicos
    "Win32_Storage_FileSystem",  # CreateFileW, GetVolumeInformationW
    "Win32_System_Ioctl",        # DeviceIoControl (USN Journal)
    "Win32_System_IO",           # Overlapped I/O
    "Win32_System_Pipes",        # CreateNamedPipeW, ConnectNamedPipe
    "Win32_Security",            # SECURITY_ATTRIBUTES (NULL DACL)
    "Win32_System_Threading",    # CreateEventW, WaitForSingleObject
]
```

### Protocolo IPC (mtt-search-protocol)
```toml
mtt-search-protocol = { path = "../mtt-search-protocol" }
```
- **Propósito**: Tipos compartilhados para comunicação via Named Pipes
- **Dependências**: `serde`, `bincode`

## Build Dependencies

```toml
[build-dependencies]
winresource = "0.1"
```
- **Propósito**: Incluir recursos Windows no executável
- **Uso**: Ícone do aplicativo, manifest

## Dev Dependencies

```toml
[dev-dependencies]
criterion = "0.5"
```
- **Propósito**: Framework de benchmarks
- **Uso**: Benchmarks de performance (`cargo bench`)

## Profile de Release

```toml
[profile.release]
opt-level = 3      # Otimização máxima
lto = true         # Link Time Optimization
codegen-units = 1  # Single codegen unit (melhor otimização)
```

### Impacto no Build
- **Build time**: Mais lento (LTO + single codegen unit)
- **Binary size**: Menor (LTO remove código não usado)
- **Performance**: Máxima (opt-level 3)

## Dependências de Runtime

### Obrigatórias
| Dependência | Versão | Onde Obter |
|-------------|--------|------------|
| libmpv-2.dll | Latest | https://sourceforge.net/projects/mpv-player-windows/files/libmpv/ |

### Opcionais (mas recomendadas)
| Dependência | Versão | Onde Obter |
|-------------|--------|------------|
| WebView2 Runtime | Latest | `winget install Microsoft.EdgeWebView2Runtime` |

## Árvore de Dependências Simplificada

```
[workspace]
├── mtt-file-manager (app GUI)
│   ├── eframe 0.31
│   │   ├── egui
│   │   ├── winit
│   │   └── ...
│   ├── windows 0.61.0
│   ├── rusqlite 0.32 (workspace)
│   ├── mtt-search-protocol (workspace)
│   ├── serde + bincode (workspace)
│   ├── image 0.25
│   ├── libmpv2 5.0.3
│   ├── rayon 1.10
│   ├── crossbeam-channel 0.5.15
│   └── ... (outras)
├── mtt-search-protocol (lib IPC)
│   ├── serde 1.0 (workspace)
│   └── bincode 1.3 (workspace)
└── mtt-search-service (Windows Service)
    ├── mtt-search-protocol
    ├── windows 0.61.0 (features mínimas)
    ├── windows-service 0.7
    ├── rusqlite 0.32 (workspace)
    ├── serde + bincode (workspace)
    └── ...
```

## Atualização de Dependências

### Verificar Updates
```bash
# Instalar cargo-outdated
cargo install cargo-outdated

# Verificar updates disponíveis
cargo outdated
```

### Atualizar
```bash
# Atualizar todas as dependências
cargo update

# Atualizar crate específico
cargo update -p eframe
```

### Segurança
```bash
# Verificar vulnerabilidades
cargo install cargo-audit
cargo audit
```

## Notas de Compatibilidade

### Windows-rs 0.61
- Crate estável com bindings atualizados
- Requer Windows 10/11 SDK durante build
- Features selecionadas manualmente para reduzir tempo de compilação

### Eframe/Egui 0.31
- Versão estáclia com API consistente
- Persistence feature para salvar estado da janela
- Suporte a wgpu/opengl backends

### Libmpv2
- Requer DLL no runtime
- Versão da DLL deve ser compatível com bindings
- Testar reprodução de vídeo após atualização

---

*Última atualização: 2026-02-11 (adicionadas dependências do serviço de busca e workspace)*
