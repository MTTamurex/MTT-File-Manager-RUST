# MTT File Manager

**Gerenciador de arquivos nativo para Windows** desenvolvido em Rust com interface moderna e recursos avançados de visualização de mídia.

## O que é

Um gerenciador de arquivos desktop para Windows que oferece:

- **Interface moderna** com janela borderless customizada
- **Navegação em abas** com histórico independente por aba
- **Preview de mídia** integrado (imagens, vídeos, GIFs, PDFs)
- **Thumbnails inteligentes** com cache em disco e múltiplos backends
- **Menus de contexto nativos** do Windows Shell
- **Integração com Lixeira** do Windows
- **Suporte a OneDrive** (detecção de status de sincronização)

## Tecnologias

| Categoria | Tecnologia |
|-----------|------------|
| Linguagem | Rust (Edition 2021) |
| GUI Framework | eframe/egui 0.31 |
| Windows API | windows crate 0.61 |
| Vídeo | libmpv2 (mpv) |
| PDF | WebView2 (Edge) |
| Cache | SQLite (rusqlite), LRU |
| Imagens | image crate, WIC, Media Foundation |
| Paralelismo | rayon |

## Como Compilar

### Pré-requisitos

- Rust toolchain (via rustup)
- MSVC Build Tools
- Windows 10/11 SDK

### Build

```powershell
# Clone o repositório
git clone <url>
cd MTT-File-Manager-RUST

# Build de desenvolvimento
cargo build

# Build otimizado para produção
cargo build --release
```

### Executar

```powershell
# Desenvolvimento
cargo run

# Produção
cargo run --release

# Ou diretamente
.\target\release\mtt-file-manager.exe
```

## Estrutura do Projeto

```
src/
├── app/           # Estado e operações da aplicação
├── application/   # Serviços de lógica de negócio
├── domain/        # Modelos de dados
├── infrastructure/# Windows API, cache, mídia
├── ui/            # Componentes de interface
├── workers/       # Threads de background
├── tabs/          # Sistema de abas
└── pdf_viewer/    # Visualizador externo de PDF
```

Para detalhes completos, veja `/docs/project-structure.md`.

## Documentação Técnica

| Documento | Descrição |
|-----------|-----------|
| [technologies.md](docs/technologies.md) | Catálogo completo de tecnologias |
| [architecture.md](docs/architecture.md) | Arquitetura e fluxos de dados |
| [project-structure.md](docs/project-structure.md) | Estrutura de arquivos e pastas |
| [modules-and-functions.md](docs/modules-and-functions.md) | Módulos e funções principais |
| [configuration.md](docs/configuration.md) | Configuração e ambiente |
| [build-and-run.md](docs/build-and-run.md) | Build, execução e deploy |
| [risks-and-unknowns.md](docs/risks-and-unknowns.md) | Riscos e pontos de atenção |

## Status do Projeto

**Em desenvolvimento ativo**

O projeto está funcional mas em evolução. Principais funcionalidades implementadas:

- ✅ Navegação de arquivos (Grid e Lista)
- ✅ Preview de imagens e GIFs
- ✅ Reprodução de vídeo com mpv
- ✅ Visualização de PDF
- ✅ Operações de arquivo (copiar, colar, deletar, renomear)
- ✅ Menu de contexto nativo do Windows
- ✅ Lixeira do Windows
- ✅ Multi-abas
- ✅ Thumbnails com cache persistente

## Limitações Conhecidas

1. **Windows only** - Não há suporte para Linux/macOS
2. **Dependência de mpv** - Requer `libmpv-2.dll` para vídeo
3. **Dependência de WebView2** - Requer Edge WebView2 Runtime para PDF
4. **Idioma** - Interface em Português (BR) hardcoded
5. **Testes** - Cobertura mínima de testes automatizados

## Dependências Externas

- **libmpv**: Biblioteca do mpv player (incluso `mpv.lib`, runtime DLL necessária)
- **WebView2**: Microsoft Edge WebView2 Runtime
- **Fontes**: Segoe UI (Windows system font)

## Licença

⚠️ **Não documentada no código atual**
