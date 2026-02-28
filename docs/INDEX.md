# Índice de Documentação - MTT File Manager

Este índice fornece navegação para todos os documentos técnicos do MTT File Manager.

## Documentos Principais

### [01_overview.md](01_overview.md)
**Visão Geral do Projeto**
- O que é o MTT File Manager
- Principais recursos e capacidades
- Arquitetura de alto nível
- Tecnologias utilizadas
- Limitações conhecidas

### [02_build_run_debug.md](02_build_run_debug.md)
**Build, Execução e Debug**
- Pré-requisitos e instalação
- Como compilar (dev e release)
- Como executar com logs
- Debug e profiling
- Solução de problemas comuns

### [03_architecture.md](03_architecture.md)
**Arquitetura do Sistema**
- Estrutura do Cargo Workspace (3 crates)
- Camadas e responsabilidades (UI, Application, Domain, Infrastructure)
- Serviço de Busca Global (processo externo com indexação híbrida: USN + full-scan fallback)
- Visualizador de Imagens Dedicado (processo separado com cache sliding-window)
- Principais boundaries
- Ciclo de vida da aplicação
- Estado global e gerenciamento
- Workers e comunicação entre threads
- Pontos de extensão

### [04_module_map.md](04_module_map.md)
**Mapa do Repositório**
- Estrutura de diretórios completa (workspace com 3 crates)
- Responsabilidades por módulo
- Crates: mtt-search-protocol e mtt-search-service
- Visualizador de imagens dedicado (`src/image_viewer/`)
- Principais structs e funções
- Dependências entre módulos
- Fluxo de dados principal

### [05_dependencies_stack.md](05_dependencies_stack.md)
**Stack de Dependências**
- Dependências do Cargo.toml
- Features e versionamento
- Integrações com Windows
- Dependências de runtime
- Alternativas consideradas

### [06_key_flows.md](06_key_flows.md)
**Fluxos Principais**
- Navegação para pasta
- Preview de arquivo (imagem, vídeo, PDF, GIF)
- Operações de arquivo (copiar, mover, deletar)
- Geração de thumbnail (multi-estágio)
- Menu de contexto
- Lixeira
- Navegação por teclado
- Busca global (Ctrl+Shift+F → Named Pipe → índice híbrido USN/fallback)
- Folder cover composition (composição customizada com layers PNG)
- Visualizador de imagens dedicado (processo separado, cache sliding-window, prefetch)
- Debugging por fluxo

### [07_storage_config.md](07_storage_config.md)
**Configuração e Storage**
- Localização dos arquivos (%LOCALAPPDATA% e %PROGRAMDATA%)
- Banco de dados SQLite do app (schema, operações)
- Banco de dados SQLite do serviço de busca (índice de arquivos)
- Configurações e preferências
- Cache de thumbnails (WebP)
- Cache em memória (LRU, DashMap)
- Como resetar/limpar dados

### [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md)
**Logs, Erros e Telemetria**
- Sistema de logs atual (eprintln!)
- Categorias de logs
- Como capturar logs (PowerShell scripts)
- Sistema de erros AppError
- Helpers e macros de erro
- Stack traces e backtraces
- Debugging avançado

### [09_support_playbook.md](09_support_playbook.md)
**Playbook de Suporte**
- Checklist de triagem

### [10_file_pilot_optimizations.md](10_file_pilot_optimizations.md)
**Otimizações do File Pilot**
- NtQueryDirectoryFile para indexação rápida
- Drive-wide file watching (ReadDirectoryChangesW)
- Smart DELETE handling
- Arquitetura e integração
- Problemas comuns e soluções
- Perguntas padrão para tickets
- Scripts de diagnóstico
- Procedimentos de escalação

## Navegação Rápida por Tópico

### Para Novos Desenvolvedores
1. Comece com [01_overview.md](01_overview.md) para entender o que é
2. Veja [02_build_run_debug.md](02_build_run_debug.md) para configurar ambiente
3. Leia [03_architecture.md](03_architecture.md) para entender a arquitetura
4. Use [04_module_map.md](04_module_map.md) para navegar o código

### Para Debug e Suporte
1. Use [09_support_playbook.md](09_support_playbook.md) para triagem inicial
2. Veja [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) para capturar logs
3. Consulte [06_key_flows.md](06_key_flows.md) para debugging por fluxo
4. Use [07_storage_config.md](07_storage_config.md) para problemas de cache/config

### Para Entender Dependências
1. Leia [05_dependencies_stack.md](05_dependencies_stack.md) para stack completo
2. Verifique [04_module_map.md](04_module_map.md) para dependências por módulo

### Para Problemas de Performance
1. Use [08_logging_errors_telemetry.md](08_logging_errors_telemetry.md) para métricas
2. Veja [06_key_flows.md](06_key_flows.md) para pontos de performance
3. Consulte [07_storage_config.md](07_storage_config.md) para cache optimization

## Arquivos de Código Importantes

### Entry Points
- `src/main.rs` - Ponto de entrada do aplicativo
- `src/lib.rs` - Ponto de entrada da biblioteca
- `src/app/state.rs` - Estado principal da aplicação (ImageViewerApp)
- `src/app/init.rs` - Inicialização da aplicação
- `crates/mtt-search-service/src/main.rs` - Entry point do serviço de busca
- `crates/mtt-search-protocol/src/lib.rs` - Tipos IPC compartilhados

### Fluxos Principais
- `src/app/operations/navigation/` - Navegação (mod.rs, keyboard.rs, selection.rs)
- `src/app/operations/folder_loading/mod.rs` - Carregamento de pastas
- `src/app/operations/message_handler/mod.rs` - Processamento de eventos assíncronos
- `src/app/operations/thumbnails.rs` - Solicitação de thumbnails
- `src/workers/thumbnail/` - Workers de thumbnail (multi-estágio)
- `src/ui/app_impl.rs` - Main loop da UI (orquestração)
- `src/ui/views/grid_view/mod.rs` - Renderização de grid modularizada

### Integrações Críticas
- `src/infrastructure/windows/` - Integrações Windows (Shell, COM, Media Foundation)
- `src/infrastructure/global_search.rs` - Cliente IPC para serviço de busca (Named Pipe)
- `src/infrastructure/disk_cache.rs` - Cache em disco (SQLite)
- `src/ui/cache.rs` - Cache de texturas GPU
- `src/workers/` - Workers assíncronos
- `src/workers/global_search_worker.rs` - Worker de busca global

### Sistema de Preview
- `src/ui/preview_panel/` - Painel de preview
- `src/ui/components/media_preview.rs` - Preview de mídia
- `src/ui/components/mpv_preview/mod.rs` - Preview de vídeo
- `src/ui/components/item_slot/mod.rs` - Renderização de slot de item (grid)
- `src/pdf_viewer/` - Visualizador de PDF nativo (processo separado, Windows.Data.Pdf)
- `src/image_viewer/` - Visualizador de imagens dedicado (processo separado)
  - `src/image_viewer/mod.rs` - Entry points (open_image_viewer, run_standalone)
  - `src/image_viewer/app.rs` - App struct, UI, navegação, zoom
  - `src/image_viewer/cache.rs` - WindowCache + PrefetchEngine
  - `src/image_viewer/indexer.rs` - Leitura e ordenação do diretório
  - `src/image_viewer/loader.rs` - Decodificação de imagens (mmap, EXIF, WIC)

## Comandos Úteis

### Build e Execução
```bash
# Desenvolvimento (workspace completo)
cargo build --workspace
cargo run

# Produção
cargo build --release --workspace

# App com logs (PowerShell)
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "debug.log"

# Serviço de busca em modo console
.\target\release\mtt-search-service.exe run-console

# Sem fallback notify (UNC/rede)
cargo build --no-default-features
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
```

### Limpeza e Reset
```powershell
# Limpar cache
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force

# Limpar build
cargo clean

# Limpar tudo
cargo clean
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

## Estrutura de Diretórios do Projeto

```
MTT-File-Manager-RUST/
├── Cargo.toml                        # Workspace root + app principal
├── src/                              # App principal (mtt-file-manager)
│   ├── app/                          # Estado e lógica principal
│   │   ├── operations/               # Handlers de operações
│   │   │   ├── navigation/           # Navegação
│   │   │   ├── ui_rendering/         # Renderização UI
│   │   │   └── *.rs                  # Outros handlers
│   │   ├── state.rs                  # ImageViewerApp principal
│   │   ├── init.rs                   # Inicialização
│   │   └── *_state.rs                # Sub-estados
│   ├── application/                  # Lógica de negócios
│   ├── domain/                       # Modelos de dados
│   ├── infrastructure/               # Integrações Windows
│   │   ├── global_search.rs          # Cliente IPC (Named Pipe)
│   │   └── ...
│   ├── ui/                           # Interface do usuário
│   │   ├── global_search_overlay.rs  # Overlay de busca global
│   │   └── ...
│   ├── workers/                      # Threads de background
│   │   ├── global_search_worker.rs   # Worker de busca global
│   │   └── ...
│   ├── pdf_viewer/                   # Visualizador PDF nativo (processo separado)
│   │   ├── mod.rs                     # Entry points (open_pdf_viewer, run_standalone)
│   │   ├── renderer.rs                # PdfRenderer (Windows.Data.Pdf WinRT)
│   │   ├── render_worker.rs           # Thread de renderização assíncrona
│   │   ├── viewer_app.rs              # PdfViewerApp (scroll, zoom, rotação)
│   │   └── toolbar.rs                 # Toolbar (navegação, zoom, rotação)
│   ├── image_viewer/                  # Visualizador de imagens (processo separado)
│   │   ├── mod.rs                     # Entry points
│   │   ├── app.rs                     # App struct + UI + navegação
│   │   ├── cache.rs                   # WindowCache + PrefetchEngine
│   │   ├── indexer.rs                 # Leitura de diretório
│   │   └── loader.rs                  # Decodificação (mmap, EXIF, WIC)
│   ├── tabs/                          # Sistema de abas
│   ├── lib.rs                        # Entry point lib
│   └── main.rs                       # Entry point bin
├── crates/
│   ├── mtt-search-protocol/          # Tipos IPC compartilhados
│   └── mtt-search-service/           # Windows Service de indexação híbrida
│       └── src/
│           ├── main.rs               # Entry point + orquestração
│           ├── usn_journal.rs        # Descoberta de volumes + API USN
│           ├── fs_walker.rs          # Full scan para volumes sem USN
│           ├── file_index.rs         # Índice in-memory
│           ├── path_resolver.rs      # Reconstrução de paths
│           ├── index_db.rs           # Persistência SQLite
│           ├── ipc_server.rs         # Named Pipe server
│           └── service_control.rs    # Install/uninstall
└── docs/                             # Documentação técnica
```

## Status da Documentação

✅ **Documentação Completa** - Todos os documentos principais atualizados  
✅ **Arquitetura Documentada** - Camadas e fluxos descritos  
✅ **Dependências Mapeadas** - Stack completa documentada  
✅ **Fluxos Detalhados** - Principais fluxos documentados  
✅ **Playbook de Suporte** - Procedimentos de suporte definidos  

## Notas e Limitações

### Documentação Futura (Não Implementada)
- API pública (não existe atualmente)
- Plugins/extensions (não implementado)
- Temas customizáveis (não implementado)
- Internacionalização (apenas PT-BR)

### Áreas que Precisam de Atenção
- Testes automatizados são mínimos
- Documentação de APIs internas poderia ser mais detalhada
- Guias de contribuição poderiam ser adicionados
- Documentação de deployment/instalação

## Links Externos Importantes

### Rust e Crates
- [Rust Documentation](https://doc.rust-lang.org/)
- [egui Documentation](https://docs.rs/egui/)
- [windows-rs Documentation](https://microsoft.github.io/windows-docs-rs/)

### Windows APIs
- [Windows API Documentation](https://docs.microsoft.com/windows/win32/)
- [Windows Shell Documentation](https://docs.microsoft.com/windows/win32/shell/)
- [Media Foundation](https://docs.microsoft.com/windows/win32/medfound/)

### Dependências Externas
- [mpv/libmpv](https://mpv.io/)
- [Windows.Data.Pdf API](https://learn.microsoft.com/uwp/api/windows.data.pdf)

---

*Última atualização: 2025-06-26 (PDF viewer migrado de WebView2 para Windows.Data.Pdf nativo)*
