# MTT File Manager

<div align="center">

![MTT File Manager](assets/icons/screenshot_placeholder.png)

**Gerenciador de arquivos moderno e de alta performance para Windows, desenvolvido em Rust com interface Immediate Mode (egui).**

[![Rust](https://img.shields.io/badge/Rust-1.75+-orange.svg)](https://www.rust-lang.org/)
[![Windows](https://img.shields.io/badge/Platform-Windows%2010%2F11-blue.svg)](https://www.microsoft.com/windows)
[![License](https://img.shields.io/badge/License-MIT-green.svg)](LICENSE)

</div>

---

## 📸 Screenshot

![Screenshot da Interface](docs/screenshot.png)

> *Adicione uma captura de tela da aplicação em `docs/screenshot.png`*

---

## ✨ Funcionalidades

### 🖼️ Visualização de Mídia
- **Thumbnails de Imagens**: PNG, JPG, WEBP, HEIC, AVIF, BMP, GIF, TIFF, ICO
- **Thumbnails de Vídeos**: MP4, MKV, AVI, MOV, WMV, WEBM, M4V, 3GP, TS
- **API Nativa do Windows**: `IShellItemImageFactory` + WIC + Media Foundation
- **Preview Nativo de Pastas**: Efeito "sanduíche" como o Windows Explorer

### ⚡ Performance
- **Carregamento Assíncrono**: Interface nunca congela (60+ FPS garantidos)
- **Lazy Loading**: Thumbnails carregados sob demanda (viewport visível)
- **Worker Pool**: 4 workers de thumbnail com controle de concorrência
- **Cache Inteligente**: LRU em memória + SQLite persistente (WebP comprimido)
- **Ordenação Paralela**: Usa `rayon` para listas >5000 itens

### 🎨 Interface
- **Grid & List Views**: Alterna entre visualização em grade e lista
- **Zoom Dinâmico**: 64px até 256px com slider
- **Sistema de Abas**: Múltiplas pastas abertas simultaneamente
- **Sidebar**: Acesso rápido a drives, OneDrive e Lixeira
- **Breadcrumbs**: Navegação visual por caminho
- **Barra de Detalhes**: Preview, metadados de mídia, informações de arquivo

### 📂 Gerenciamento de Arquivos
- **Operações Shell**: Copiar, Recortar, Colar, Renomear, Excluir (com Undo)
- **Clipboard Windows**: Integração nativa CF_HDROP
- **Menu de Contexto**: Extensões de shell do Windows (Open With, Properties, etc.)
- **File Watcher**: Auto-refresh ao detectar mudanças no sistema de arquivos
- **Lixeira**: Visualização e restauração de itens excluídos

### 🔧 Integração Windows
- **OneDrive**: Detecção de status de sincronização (Cloud/Local/Syncing)
- **Drives de Rede**: Suporte completo a unidades mapeadas
- **Ícones Nativos**: Extração via Shell API para todos os tipos de arquivo
- **Metadados de Mídia**: Resolução, duração, bitrate, codec via Media Foundation

---

## 🛠️ Tecnologias

| Componente | Tecnologia |
|------------|------------|
| **Linguagem** | Rust 1.75+ |
| **GUI Framework** | egui 0.31 + eframe |
| **APIs Windows** | windows-rs 0.58 (Win32 Shell, COM, Media Foundation) |
| **Cache** | SQLite (rusqlite) + LRU em memória |
| **Processamento** | rayon (paralelo), image (codec), webp |
| **File Watch** | notify 6.x |

---

## 📦 Instalação

### Pré-requisitos
- **Windows 10** (build 1809+) ou **Windows 11**
- **Rust** (stable 1.75+): [rustup.rs](https://rustup.rs/)
- **Visual Studio Build Tools** (para compilação)

### Build de Desenvolvimento

```powershell
# Clone o repositório
git clone https://github.com/seu-usuario/mtt-file-manager.git
cd mtt-file-manager

# Compile e execute
cargo run
```

### Build de Produção

```powershell
# Build otimizado com LTO
cargo build --release

# Executável gerado em:
.\target\release\mtt-file-manager.exe
```

---

## ⚙️ Configuração de Build

O projeto usa otimizações agressivas para produção (`Cargo.toml`):

```toml
[profile.release]
opt-level = 3        # Máxima otimização
lto = true           # Link-Time Optimization
codegen-units = 1    # Melhor otimização cross-crate
```

---

## 🎹 Atalhos de Teclado

| Atalho | Ação |
|--------|------|
| `F5` | Atualizar pasta atual |
| `Enter` | Abrir item selecionado |
| `Delete` | Excluir item (vai para Lixeira) |
| `Ctrl+C` | Copiar |
| `Ctrl+X` | Recortar |
| `Ctrl+V` | Colar |
| `Ctrl+Shift+N` | Nova pasta |
| `F2` | Renomear |
| `←` `→` `↑` `↓` | Navegar entre itens |
| `Backspace` | Voltar nível |

---

## 🏗️ Arquitetura

### Estrutura de Módulos

```
src/
├── main.rs                # Aplicação principal (~5000 linhas)
├── lib.rs                 # Re-exports dos módulos
├── domain/                # Entidades e regras de domínio
│   ├── file_entry.rs      # FileEntry, SortMode, ViewMode
│   └── thumbnail.rs       # ThumbnailData
├── application/           # Orquestração e casos de uso
│   ├── state.rs           # AppState (gestão de estado)
│   ├── clipboard.rs       # Operações de clipboard
│   ├── context_menu.rs    # Estado do menu de contexto
│   ├── navigation.rs      # Histórico de navegação
│   └── notification.rs    # Sistema de toasts
├── infrastructure/        # Implementações de baixo nível
│   ├── disk_cache.rs      # Cache SQLite persistente
│   ├── onedrive.rs        # Integração OneDrive
│   └── windows/           # APIs Win32
│       ├── icons.rs       # Extração de ícones
│       ├── shell_operations.rs
│       └── media_foundation.rs
├── workers/               # Background threads
│   ├── thumbnail_worker.rs # Pool de workers (4 threads)
│   └── folder_preview_worker.rs
└── ui/                    # Componentes de interface
    ├── views/             # Grid, List, Computer view
    ├── components/        # Item slots, sidebar
    └── cache.rs           # Cache de texturas LRU
```

### Diagrama de Fluxo

```
┌─────────────────────────────────────────────────────────────────┐
│                         UI Thread (egui)                        │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────────────┐   │
│  │ Sidebar │  │ Toolbar │  │ Grid/   │  │ Preview Panel   │   │
│  │         │  │         │  │ List    │  │                 │   │
│  └────┬────┘  └────┬────┘  └────┬────┘  └────────┬────────┘   │
│       │            │            │                 │            │
│       └────────────┴─────┬──────┴─────────────────┘            │
│                          │                                      │
│                    ┌─────▼─────┐                               │
│                    │  State    │                               │
│                    │  Manager  │                               │
│                    └─────┬─────┘                               │
└──────────────────────────┼─────────────────────────────────────┘
                           │ mpsc channels
          ┌────────────────┼────────────────┐
          ▼                ▼                ▼
    ┌──────────┐    ┌──────────┐    ┌──────────┐
    │Thumbnail │    │  Folder  │    │ Metadata │
    │ Workers  │    │  Scanner │    │  Worker  │
    │  (4x)    │    │          │    │          │
    └────┬─────┘    └────┬─────┘    └────┬─────┘
         │               │               │
         ▼               ▼               ▼
    ┌─────────────────────────────────────────┐
    │           Windows APIs                   │
    │  Shell • WIC • Media Foundation • COM   │
    └─────────────────────────────────────────┘
```

---

## 📄 Licença

MIT License - Veja [LICENSE](LICENSE) para detalhes.

---

## 🙏 Agradecimentos

- **[egui](https://github.com/emilk/egui)** - Framework de UI Immediate Mode
- **[windows-rs](https://github.com/microsoft/windows-rs)** - Bindings oficiais Microsoft
- **[rayon](https://github.com/rayon-rs/rayon)** - Paralelismo data-parallel
- **[rusqlite](https://github.com/rusqlite/rusqlite)** - SQLite bindings

---

<div align="center">

Feito com ❤️ em Rust

</div>

- **Bugs**: Abra uma [Issue no GitHub](https://github.com/seu-usuario/mtt-file-manager/issues)
- **Features**: Consulte [ROADMAP_TECNICO.md](docs/ROADMAP_TECNICO.md) e vote/comente
- **Discussões**: [GitHub Discussions](https://github.com/seu-usuario/mtt-file-manager/discussions)
- **Segurança**: Reporte vulnerabilidades diretamente aos maintainers (NÃO abra issue pública)

---

**Última Atualização**: 2025-12-27  
**Versão**: 0.1.0  
**Status**: ⚠️ Alpha (uso em produção não recomendado ainda)
