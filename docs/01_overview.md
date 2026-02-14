# MTT File Manager - Visão Geral

## Objetivo do Documento
Este documento fornece uma visão geral do MTT File Manager, suas capacidades principais e arquitetura de alto nível.

## O que é o MTT File Manager

O MTT File Manager é um gerenciador de arquivos nativo para Windows desenvolvido em Rust, com interface moderna e recursos avançados de visualização de mídia. O aplicativo oferece uma experiência de usuário fluída com navegação em abas, preview integrado de arquivos e integração profunda com o Windows.

## Principais Recursos

### Navegação e Interface
- **Interface borderless customizada** - Janela sem bordas tradicionais com suporte nativo para redimensionamento
- **Navegação em abas** - Sistema de abas com histórico independente por aba
- **Visualizações múltiplas** - Modo grade e lista com thumbnails ajustáveis
- **Barra de endereços editável** - Navegação direta por caminhos com breadcrumbs
- **Sidebar com atalhos** - Acesso rápido a drives, bibliotecas, OneDrive e Lixeira
- **Suporte a navegação por teclado** - Atalhos completos para navegação sem mouse
- **Busca em tempo real** - Filtro de arquivos por nome

### Preview e Mídia
- **Preview integrado** - Visualização de imagens, vídeos, GIFs e PDFs sem sair do aplicativo
- **Reprodução de vídeo** - Player baseado em libmpv para formatos de vídeo diversos com suporte a controles, tela cheia e janela destacada
- **Visualizador de PDF** - Integração com WebView2 (Edge) para PDFs
- **Thumbnails inteligentes** - Geração e cache de thumbnails com múltiplos backends (image crate, WIC, Shell API, Media Foundation)
- **Suporte a GIFs animados** - Reprodução otimizada de GIFs com controles de play/pause
- **Extração de metadados** - EXIF de imagens, metadados de vídeo e áudio

### Operações de Arquivo
- **Operações básicas** - Copiar, cortar, colar, renomear, deletar
- **Menu de contexto nativo** - Integração com o menu de contexto do Windows Shell
- **Lixeira do Windows** - Integração completa com a lixeira do sistema (navegação, restauração, exclusão permanente)
- **Suporte a OneDrive** - Detecção de status de sincronização (cloud-only, syncing, pinned, locally available)
- **Montagem de ISO** - Suporte para montar arquivos ISO como drives virtuais
- **Renomeação inline** - Renomeação de arquivos diretamente na lista

### Busca Global
- **Overlay dedicado** - Busca global via Ctrl+Shift+F
- **Serviço externo** - `mtt-search-service` com IPC por Named Pipe
- **Indexação híbrida por volume** - NTFS/ReFS com USN Journal; volumes sem USN com varredura full-tree
- **Atualização por tipo de filesystem** - USN incremental (2s), sem USN por re-scan periódico (30s/120s)

### Sistema de Cache e Performance
- **Cache em disco** - SQLite para thumbnails e metadados
- **Cache em memória** - LRU cache para acesso rápido
- **Workers assíncronos** - Processamento em background para manter UI responsiva
- **Pré-carregamento inteligente** - Prefetch de pastas e thumbnails
- **Indexação de diretórios** - Cache de estrutura de diretórios para navegação rápida
- **Virtualização de listas** - Renderização eficiente para pastas com muitos arquivos
- **Geração de thumbnails em estágios** - Fallback progressivo: image crate → WIC → Shell API → Media Foundation

## Arquitetura de Alto Nível

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Layer                            │
│  ┌─────────────┬──────────────┬───────────────────────┐   │
│  │   Toolbar   │   Tab Bar     │    Preview Panel    │   │
│  │  Sidebar    │   File List   │    Status Bar       │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────┐
│                    Application Layer                       │
│  ┌─────────────┬──────────────┬───────────────────────┐   │
│  │Navigation   │ File Ops     │  Clipboard Manager   │   │
│  │History      │ Sorting      │  Notification System │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────┐
│                   Domain Layer                             │
│  ┌─────────────┬──────────────┬───────────────────────┐   │
│  │FileEntry    │ Thumbnail    │  Error Types         │   │
│  │Enums        │ Metadata     │  App State           │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                               │
┌─────────────────────────────────────────────────────────────┐
│                 Infrastructure Layer                         │
│  ┌─────────────┬──────────────┬───────────────────────┐   │
│  │Windows API  │ Disk Cache   │  Media Foundation    │   │
│  │Shell Integ. │ SQLite       │  Thumbnail Workers   │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Tecnologias Principais

| Categoria | Tecnologia | Versão | Propósito |
|-----------|------------|---------|-----------|
| Linguagem | Rust | 2021 Edition | Core language |
| GUI Framework | eframe/egui | 0.31 | Interface gráfica (immediate mode) |
| Windows API | windows-rs | 0.61.0 | Integração Windows |
| Cache | SQLite (rusqlite) | 0.32 | Persistência de thumbnails |
| Vídeo | libmpv2 | 5.0.3 | Reprodução de vídeo |
| PDF | WebView2 | - | Visualização de PDFs |
| Imagens | image crate | 0.25 | Processamento de imagens (WebP, GIF) |
| SVG | resvg/usvg | 0.44 | Renderização de SVG |
| Paralelismo | rayon | 1.10 | Processamento paralelo |
| Comunicação | crossbeam-channel | 0.5.15 | Canais MPSC de alta performance |
| Hashing | rustc-hash/fxhash | 2.0/0.2.1 | Hash rápida para PathBuf |
| EXIF | kamadak-exif | 0.5 | Leitura de metadados JPEG |
| Compressão | webp | 0.3 | Compressão WebP para thumbnails |
| Clipboard | clipboard-win | 5.4 | Integração clipboard Windows |
| File Dialogs | rfd | 0.15 | Diálogos de arquivo nativos |
| Watcher | Drive Watcher nativo + notify (fallback) | nativo/6.1.1 | Monitoramento de filesystem (local + UNC) |

## Dependências de Runtime

- **libmpv-2.dll** - Necessária para reprodução de vídeo
- **Microsoft Edge WebView2 Runtime** - Necessário para visualização de PDFs

## Limitações Conhecidas

1. **Windows Only** - Não há suporte para Linux/macOS devido às dependências de Windows API
2. **Dependência de mpv** - Requer `libmpv-2.dll` para reprodução de vídeo
3. **Dependência de WebView2** - Requer Microsoft Edge WebView2 Runtime para PDFs
4. **Idioma** - Interface em Português (BR) hardcoded
5. **Testes** - Cobertura mínima de testes automatizados

## Requisitos do Sistema

### Mínimos
- Windows 10 ou superior
- 4GB RAM
- 100MB espaço em disco

### Recomendados
- Windows 11
- 8GB RAM ou mais
- SSD para melhor performance de cache
- Placa de vídeo dedicada para preview de vídeos

## Onde Encontrar Mais Informações

- [02_build_run_debug.md](02_build_run_debug.md) - Como compilar e executar
- [03_architecture.md](03_architecture.md) - Arquitetura detalhada
- [04_module_map.md](04_module_map.md) - Mapa dos módulos
- [05_dependencies_stack.md](05_dependencies_stack.md) - Stack de dependências
- [06_key_flows.md](06_key_flows.md) - Fluxos principais
- [07_storage_config.md](07_storage_config.md) - Configurações e storage
- [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) - Logs e erros
- [09_support_playbook.md](09_support_playbook.md) - Playbook de suporte

---

*Última atualização: 2026-02-14 (adicionado resumo da busca híbrida com fallback sem USN)*
