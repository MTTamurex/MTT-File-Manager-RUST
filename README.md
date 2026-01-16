# MTT File Manager

Um gerenciador de arquivos moderno e eficiente para Windows, desenvolvido em Rust com interface gráfica usando egui/eframe e **player de vídeo integrado via MPV**.

> [!IMPORTANT]
> **Pré-requisito de Sistema (MPV)**: Forneça `mpv-1.dll` ao lado do executável (ou no PATH).

---

## 🚀 Características

- **Interface moderna** com suporte a temas claro/escuro
- **Navegação por abas** para múltiplos diretórios
- **Visualização em grade e lista** com miniaturas
- **Player de vídeo integrado** (MPV) com controles nativos
- **Preview de arquivos** (imagens, vídeos, GIFs animados)
- **Integração nativa com Windows** (clipboard, menus de contexto, shell extensions)
- **Suporte a OneDrive** com indicadores de status
- **Metadados de mídia** (dimensões, duração, codec, bitrate)
- **Operações de arquivo** (copiar, mover, renomear, excluir)
- **Lixeira do Windows** com restauração
- **Cache de miniaturas** SQLite para performance

---

## 🏗️ Arquitetura Híbrida

O MTT File Manager utiliza uma arquitetura híbrida para superar as limitações de renderização de vídeo em ambientes Rust puro usando MPV:

```
┌─────────────────────────────────────────────────────────┐
│                    MTT File Manager                      │
├─────────────────────────────────────────────────────────┤
│  ┌─────────────────────┐    ┌─────────────────────────┐ │
│  │      egui/eframe    │    │          MPV           │ │
│  │  ─────────────────  │    │  ─────────────────────  │ │
│  │  • UI Principal     │    │  • Player de Vídeo      │ │
│  │  • Navegação        │◄──►│  • Decodificação GPU    │ │
│  │  • Thumbnails       │    │  • Janela child (wid)   │ │
│  │  • Controles        │    │                         │ │
│  └─────────────────────┘    └─────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### Por que MPV?

- **Suporte amplo de formatos/containers**
- **Dispensa transcoding na maioria dos casos**
- **Integração via janela filha (wid)**

## 🛠️ Tecnologias

| Dependência | Versão | Propósito |
|-------------|--------|-----------|
| `eframe` | 0.31 | Framework egui com persistence |
| `libmpv2` | 5.0.3 | MPV para player de vídeo |
| `windows` | 0.58 | Bindings para APIs Win32 |
| `rayon` | 1.10 | Paralelismo para ordenação |
| `rusqlite` | 0.32 | Cache SQLite persistente |
| `image` | 0.25 | Decodificação de imagens |
| `notify` | 6.1.1 | File system watcher |
| `lru` | 0.12 | Cache LRU para texturas |
| `resvg/usvg` | 0.44 | Renderização de ícones SVG |

---

## 📁 Estrutura do Projeto

```
src/
├── main.rs                 # Bootstrap (135 linhas)
├── lib.rs                  # Biblioteca pública
│
├── app/                    # Lógica de aplicação
│   ├── mod.rs              # ImageViewerApp struct
│   ├── init.rs             # Inicialização
│   ├── state.rs            # Estado da aplicação
│   └── operations/         # Métodos da aplicação (19 módulos)
│       ├── clipboard_ops.rs
│       ├── context_menu.rs
│       ├── file_ops.rs
│       ├── folder_loading.rs
│       ├── navigation.rs
│       ├── tabs.rs
│       ├── thumbnails.rs
│       └── ...
│
├── ui/                     # Componentes de interface
│   ├── app/                # Implementação eframe::App
│   │   ├── input.rs
│   │   ├── lifecycle.rs
│   │   ├── panels.rs
│   │   └── ...
│   │
│   ├── components/         # Componentes reutilizáveis
│   │   ├── item_slot.rs        # Slot de item com preview
│   │   ├── media_preview.rs    # Preview de mídia (imagens/GIFs)
│   │   └── mpv_preview.rs      # Player de vídeo MPV
│   │
│   ├── views/              # Views de exibição
│   │   ├── computer_view.rs
│   │   ├── grid_view.rs
│   │   └── list_view.rs
│   │
│   └── [outros componentes]
│
├── infrastructure/         # Serviços de infraestrutura
│   ├── windows/            # Integração Windows
│   │   ├── metadata/           # Extração de metadados
│   │   │   ├── image.rs
│   │   │   └── video.rs
│   │   ├── codec_registry.rs   # Resolução de nomes de codec
│   │   ├── icons.rs            # Extração de ícones nativos
│   │   ├── native_menu.rs      # Menu de contexto nativo
│   │   ├── recycle_bin.rs      # Operações de lixeira
│   │   └── shell_operations.rs # Operações de shell
│   │
│   ├── cache.rs            # Cache de miniaturas
│   ├── disk_cache.rs       # Cache em disco (SQLite)
│   └── onedrive.rs         # Integração OneDrive
│
├── domain/                 # Entidades de domínio
│   ├── file_entry.rs
│   ├── thumbnail.rs
│   └── errors.rs
│
├── application/            # Serviços de aplicação
│   ├── clipboard.rs
│   ├── navigation.rs
│   └── state.rs
│
└── workers/                # Workers assíncronos
    ├── thumbnail_loader.rs
    ├── folder_scanner.rs
    └── folder_preview_worker.rs
```

---

## 🏗️ Build

### Requisitos

- **Rust 1.75** ou superior
- **Windows 10/11** (64-bit)
- **Visual Studio Build Tools** (para windows-rs)
- **MPV runtime** (`mpv-1.dll` ao lado do executável ou no PATH)

### Compilação

```bash
# Debug
cargo build

# Release (otimizado com LTO)
cargo build --release
```

### Execução

```bash
# Debug
cargo run

# Release
cargo run --release
```

---

## 📝 Documentação

Documentação técnica disponível em `docs/`:

| Documento | Descrição |
|-----------|-----------|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Arquitetura detalhada do sistema |
| [AUDIT_REPORT.md](docs/AUDIT_REPORT.md) | Relatório de auditoria do código |
| [MEDIA_PREVIEW_SYSTEM.md](docs/MEDIA_PREVIEW_SYSTEM.md) | Sistema de preview de mídia |
| [MEDIA_METADATA_FEATURE.md](docs/MEDIA_METADATA_FEATURE.md) | Extração de metadados |
| [CLIPBOARD_INTEGRATION.md](docs/CLIPBOARD_INTEGRATION.md) | Integração com clipboard |
| [CODEC_RESOLUTION.md](docs/CODEC_RESOLUTION.md) | Resolução de codecs |
| [PADROES_REUTILIZAVEIS.md](docs/PADROES_REUTILIZAVEIS.md) | Padrões de código |

---

## 📜 Licença

Este projeto está licenciado sob a [MIT License](LICENSE).

## 👨‍💻 Autor

Desenvolvido por MTT (Marcio T. Tamura).
