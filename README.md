# MTT File Manager

Um gerenciador de arquivos moderno e eficiente para Windows, desenvolvido em Rust com interface gráfica usando egui/eframe.

## 🚀 Características

- **Interface moderna** com suporte a temas claro/escuro
- **Navegação por abas** para múltiplos diretórios
- **Visualização em grade e lista** com miniaturas
- **Preview de arquivos** (imagens, vídeos, áudio)
- **Integração nativa com Windows** (clipboard, menus de contexto, shell extensions)
- **Suporte a OneDrive** com indicadores de status
- **Metadados de mídia** (dimensões, duração, codec, bitrate)
- **Operações de arquivo** (copiar, mover, renomear, excluir)
- **Lixeira do Windows** com restauração
- **Cache de miniaturas** para performance

## 🛠️ Tecnologias

- **Rust 1.75+** - Linguagem de programação
- **egui 0.31** - Framework de UI imediata
- **eframe** - Integração nativa com Windows
- **windows-rs 0.58** - Bindings para APIs do Windows
- **Media Foundation** - Extração de metadados de mídia

## 📁 Arquitetura do Projeto

```
src/
├── main.rs                 # Bootstrap (115 linhas)
├── lib.rs                  # Biblioteca pública
│
├── app/                    # Lógica de aplicação
│   ├── mod.rs              # ImageViewerApp struct
│   └── operations/         # Métodos da aplicação (19 módulos)
│       ├── clipboard_ops.rs    # Operações de clipboard
│       ├── context_menu.rs     # Menu de contexto
│       ├── file_ops.rs         # Operações de arquivo
│       ├── folder_loading.rs   # Carregamento de pastas
│       ├── icons.rs            # Gerenciamento de ícones
│       ├── message_handler.rs  # Processamento de mensagens
│       ├── metadata.rs         # Metadados de arquivos
│       ├── navigation.rs       # Navegação de diretórios
│       ├── preferences.rs      # Preferências do usuário
│       ├── recycle_bin_ops.rs  # Operações de lixeira
│       ├── selection.rs        # Seleção de itens
│       ├── tabs.rs             # Gerenciamento de abas
│       ├── thumbnails.rs       # Carregamento de miniaturas
│       ├── trait_impls.rs      # Implementações de traits
│       ├── ui_rendering.rs     # Renderização de UI
│       ├── view_setup.rs       # Configuração de views
│       ├── watcher.rs          # Observador de arquivos
│       └── window.rs           # Gerenciamento de janela
│
├── ui/                     # Componentes de interface
│   ├── app/                # Implementação eframe::App
│   │   ├── input.rs            # Processamento de entrada
│   │   ├── lifecycle.rs        # Ciclo de vida da aplicação
│   │   ├── menu_handler.rs     # Manipulação de menus
│   │   ├── notifications.rs    # Sistema de notificações
│   │   └── panels.rs           # Painéis principais
│   │
│   ├── components/         # Componentes reutilizáveis
│   │   └── item_slot.rs        # Slot de item com preview
│   │
│   ├── views/              # Views de exibição
│   │   ├── computer_view.rs    # View "Este Computador"
│   │   ├── grid_view.rs        # View em grade
│   │   ├── list_view.rs        # View em lista
│   │   └── common.rs           # Funções compartilhadas
│   │
│   ├── context_menu.rs     # Menu de contexto UI
│   ├── grid.rs             # Layout de grade
│   ├── header.rs           # Cabeçalho da aplicação
│   ├── icon_loader.rs      # Carregador de ícones
│   ├── navigation.rs       # Barra de navegação
│   ├── operations.rs       # Operações de UI
│   ├── sidebar.rs          # Barra lateral
│   ├── status_bar.rs       # Barra de status
│   ├── svg_icons.rs        # Ícones SVG
│   └── tab_bar.rs          # Barra de abas
│
├── infrastructure/         # Serviços de infraestrutura
│   ├── windows/            # Integração Windows
│   │   ├── metadata/           # Extração de metadados
│   │   │   ├── image.rs        # Metadados de imagem
│   │   │   ├── video.rs        # Metadados de vídeo
│   │   │   ├── property_keys.rs
│   │   │   └── utils.rs
│   │   ├── bitmap_conversion.rs
│   │   ├── codec_registry.rs
│   │   ├── device_change.rs
│   │   ├── drives.rs
│   │   ├── file_system.rs
│   │   ├── file_type.rs
│   │   ├── formatting.rs
│   │   ├── icons.rs
│   │   ├── media_foundation.rs
│   │   ├── native_menu.rs
│   │   ├── recycle_bin.rs
│   │   └── shell_operations.rs
│   │
│   ├── cache.rs            # Cache de miniaturas
│   ├── disk_cache.rs       # Cache em disco
│   ├── onedrive.rs         # Integração OneDrive
│   ├── security.rs         # Verificações de segurança
│   └── watcher.rs          # Observador de arquivos
│
├── domain/                 # Entidades de domínio
│   ├── errors.rs           # Tipos de erro
│   ├── file_entry.rs       # Entrada de arquivo
│   └── thumbnail.rs        # Miniatura
│
├── application/            # Serviços de aplicação
│   ├── clipboard.rs        # Serviço de clipboard
│   ├── context_menu.rs     # Serviço de menu de contexto
│   ├── navigation.rs       # Serviço de navegação
│   ├── notification.rs     # Serviço de notificação
│   ├── renaming.rs         # Serviço de renomeação
│   ├── state.rs            # Estado da aplicação
│   └── watcher.rs          # Serviço de observador
│
└── workers/                # Workers assíncronos
    ├── batch_thumbnail_loader.rs
    ├── folder_preview_worker.rs
    ├── folder_scanner.rs
    ├── thumbnail_loader.rs
    └── thumbnail_worker.rs
```

## 🏗️ Build

### Requisitos

- Rust 1.75 ou superior
- Windows 10/11 (64-bit)
- Visual Studio Build Tools (para windows-rs)

### Compilação

```bash
# Debug
cargo build

# Release (otimizado)
cargo build --release
```

### Execução

```bash
# Debug
cargo run

# Release
cargo run --release
```

## 📝 Documentação

Documentação técnica disponível em `docs/`:

- [AUDIT_REPORT.md](docs/AUDIT_REPORT.md) - Relatório de auditoria do código
- [CLIPBOARD_INTEGRATION.md](docs/CLIPBOARD_INTEGRATION.md) - Integração com clipboard do Windows
- [CODEC_RESOLUTION.md](docs/CODEC_RESOLUTION.md) - Resolução de codecs de mídia
- [EAC3_CODEC_FIX.md](docs/EAC3_CODEC_FIX.md) - Correção de codec EAC3
- [MEDIA_METADATA_FEATURE.md](docs/MEDIA_METADATA_FEATURE.md) - Feature de metadados de mídia
- [PADROES_REUTILIZAVEIS.md](docs/PADROES_REUTILIZAVEIS.md) - Padrões de código reutilizáveis

## 🎯 Métricas do Código

| Módulo | Linhas | Descrição |
|--------|--------|-----------|
| main.rs | 115 | Bootstrap apenas |
| app/operations/ | 19 módulos | Lógica de aplicação |
| ui/app/ | 6 módulos | Implementação eframe::App |
| ui/views/ | 4 módulos | Views de exibição |
| infrastructure/windows/ | 15+ módulos | Integração Windows |

## 📜 Licença

Este projeto está licenciado sob a [MIT License](LICENSE).

## 👨‍💻 Autor

Desenvolvido por MTT (Marcio T. Tamura).
