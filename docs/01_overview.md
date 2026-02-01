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
- **Barra de endereços editável** - Navegação direta por caminhos
- **Sidebar com atalhos** - Acesso rápido a drives, bibliotecas e OneDrive

### Preview e Mídia
- **Preview integrado** - Visualização de imagens, vídeos, GIFs e PDFs sem sair do aplicativo
- **Reprodução de vídeo** - Player baseado em libmpv para formatos de vídeo diversos
- **Visualizador de PDF** - Integração com WebView2 (Edge) para PDFs
- **Thumbnails inteligentes** - Geração e cache de thumbnails com múltiplos backends
- **Suporte a GIFs animados** - Reprodução otimizada de GIFs

### Operações de Arquivo
- **Operações básicas** - Copiar, cortar, colar, renomear, deletar
- **Menu de contexto nativo** - Integração com o menu de contexto do Windows Shell
- **Lixeira do Windows** - Integração completa com a lixeira do sistema
- **Suporte a OneDrive** - Detecção de status de sincronização
- **Montagem de ISO** - Suporte para montar arquivos ISO como drives virtuais

### Sistema de Cache e Performance
- **Cache em disco** - SQLite para thumbnails e metadados
- **Cache em memória** - LRU cache para acesso rápido
- **Workers assíncronos** - Processamento em background para manter UI responsiva
- **Pré-carregamento inteligente** - Prefetch de pastas e thumbnails

## Arquitetura de Alto Nível

```
┌─────────────────────────────────────────────────────────────┐
│                        UI Layer                            │
│  ┌─────────────┬──────────────┬───────────────────────┐   │
│  │   Toolbar   │   Tab Bar     │    Preview Panel    │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
│  ┌─────────────┬──────────────────────┬─────────────────┐   │
│  │  Sidebar    │   File List/Grid     │  Status Bar     │   │
│  └─────────────┴──────────────────────┴─────────────────┘   │
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
│  │Sorting      │ Metadata     │  App State           │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│                 Infrastructure Layer                         │
│  ┌─────────────┬──────────────┬───────────────────────┐   │
│  │Windows API  │ Disk Cache   │  Media Foundation    │   │
│  │Shell Integration│ SQLite   │  Thumbnail Workers   │   │
│  └─────────────┴──────────────┴───────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Tecnologias Principais

| Categoria | Tecnologia | Versão | Propósito |
|-----------|------------|---------|-----------|
| Linguagem | Rust | 2021 Edition | Core language |
| GUI Framework | eframe/egui | 0.31 | Interface gráfica |
| Windows API | windows-rs | 0.61 | Integração Windows |
| Cache | SQLite (rusqlite) | 0.32 | Persistência de thumbnails |
| Vídeo | libmpv2 | 5.0.3 | Reprodução de vídeo |
| PDF | WebView2 | - | Visualização de PDFs |
| Imagens | image crate | 0.25 | Processamento de imagens |
| Paralelismo | rayon | 1.10 | Processamento paralelo |

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