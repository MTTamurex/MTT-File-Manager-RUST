# MTT File Manager

**Gerenciador de arquivos nativo para Windows** desenvolvido em Rust com interface moderna, recursos avançados de visualização de mídia e integração profunda com o sistema Windows.

## 📋 Índice

- [O que é](#o-que-é)
- [Principais Recursos](#principais-recursos)
- [Tecnologias](#tecnologias)
- [Requisitos](#requisitos)
- [Instalação](#instalação)
- [Como Usar](#como-usar)
- [Documentação](#documentação)
- [Desenvolvimento](#desenvolvimento)
- [Solução de Problemas](#solução-de-problemas)
- [Contribuição](#contribuição)
- [Licença](#licença)

## 🎯 O que é

O MTT File Manager é um gerenciador de arquivos desktop que combina a performance e segurança de Rust com uma interface moderna e integração nativa com o Windows. Ele oferece uma experiência de usuário fluída com navegação em abas, preview integrado de arquivos e recursos avançados de gerenciamento.

### Problemas que Resolve
- **Visualização lenta** de imagens e vídeos em exploradores tradicionais
- **Falta de preview integrado** para múltiplos formatos de arquivo
- **Navegação ineficiente** sem suporte a abas e histórico
- **Cache inadequado** que não aproveita recursos do sistema
- **Integração limitada** com funcionalidades nativas do Windows

## ✨ Principais Recursos

### 🖥️ Interface e Navegação
- **Interface borderless customizada** - Janela moderna sem bordas tradicionais
- **Navegação em abas** - Múltiplas abas com histórico independente
- **Visualizações flexíveis** - Modo grade e lista com thumbnails ajustáveis
- **Barra de endereços inteligente** - Navegação direta com autocomplete
- **Sidebar com atalhos** - Acesso rápido a drives, bibliotecas e OneDrive

### 🎬 Preview e Mídia
- **Preview integrado** - Visualização sem sair do aplicativo
- **Reprodução de vídeo** - Player baseado em libmpv para formatos diversos
- **Visualizador de PDF** - Integração com WebView2 (Edge)
- **Thumbnails inteligentes** - Cache multi-nível com geração otimizada
- **Suporte a GIFs animados** - Reprodução otimizada e fluída

### 📁 Operações de Arquivo
- **Operações completas** - Copiar, cortar, colar, renomear, deletar
- **Menu de contexto nativo** - Integração completa com Windows Shell
- **Lixeira integrada** - Operações com lixeira do Windows
- **Suporte a OneDrive** - Detecção de status de sincronização
- **Montagem de ISO** - Suporte para arquivos ISO como drives virtuais

### ⚡ Performance e Cache
- **Cache multi-nível** - Memória, disco e GPU para performance máxima
- **Workers assíncronos** - Processamento em background sem travar UI
- **Pré-carregamento inteligente** - Prefetch preditivo de pastas e arquivos
- **Virtualização de UI** - Renderização eficiente de listas grandes
- **Otimizações NTFS** - Aproveita USN Journal para monitoramento rápido

## 🛠️ Tecnologias

| Categoria | Tecnologia | Versão | Propósito |
|-----------|------------|---------|-----------|
| **Linguagem** | Rust | 2021 Edition | Performance e segurança |
| **GUI** | eframe/egui | 0.31 | Interface gráfica moderna |
| **Windows API** | windows-rs | 0.61 | Integração nativa com Windows |
| **Vídeo** | libmpv2 | 5.0.3 | Reprodução de vídeo de alta performance |
| **PDF** | WebView2 | - | Visualização de PDFs nativa |
| **Cache** | SQLite (rusqlite) | 0.32 | Persistência confiável |
| **Imagens** | image crate | 0.25 | Processamento de imagens |
| **Paralelismo** | rayon | 1.10 | Processamento paralelo |

## 📋 Requisitos

### Mínimos
- **Sistema**: Windows 10 (Build 1903+) ou Windows 11
- **Processador**: x64, 2 cores ou mais
- **Memória**: 4GB RAM
- **Espaço**: 100MB para instalação + espaço para cache
- **GPU**: DirectX 11 compatível

### Recomendados
- **Sistema**: Windows 11 (última atualização)
- **Processador**: x64, 4+ cores
- **Memória**: 8GB RAM ou mais
- **Armazenamento**: SSD para melhor performance de cache
- **GPU**: Placa dedicada para preview de vídeos

### Dependências Externas
- **libmpv-2.dll** - Para reprodução de vídeo
- **Microsoft Edge WebView2 Runtime** - Para visualização de PDFs

## 🚀 Instalação

### Opção 1: Download Direto (Recomendado)
1. Baixe a última release de [releases](../../releases)
2. Extraia o arquivo ZIP
3. Execute `mtt-file-manager.exe`

### Opção 2: Build do Código Fonte
```bash
# Clone o repositório
git clone <url-do-repositorio>
cd MTT-File-Manager-RUST

# Build de produção
cargo build --release

# Execute
.\target\release\mtt-file-manager.exe
```

### Instalação de Dependências
```powershell
# WebView2 Runtime (se necessário)
winget install Microsoft.EdgeWebView2Runtime

# libmpv (se não incluído na release)
# Baixe de: https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
# Coloque libmpv-2.dll no mesmo diretório do executável
```

## 🎮 Como Usar

### Atalhos de Teclado Principais
- **Ctrl+T** - Nova aba
- **Ctrl+W** - Fechar aba
- **Ctrl+C/V** - Copiar/Colar
- **Delete** - Mover para lixeira
- **Shift+Delete** - Deletar permanentemente
- **F2** - Renomear
- **Ctrl+R** - Recarregar pasta
- **Alt+Enter** - Propriedades
- **Ctrl+L** - Focar barra de endereços
- **Ctrl+D** - Duplicar aba

### Dicas de Uso
1. **Thumbnail Size**: Use Ctrl+Roda do mouse para ajustar tamanho dos thumbnails
2. **Multi-seleção**: Segure Ctrl para selecionar múltiplos arquivos
3. **Preview Rápido**: Selecione um arquivo e use espaço para preview
4. **Navegação Rápida**: Use Backspace para voltar, Alt+Setas para histórico
5. **Busca**: Comece a digitar para buscar arquivos na pasta atual

### Visualização de Mídia
- **Imagens**: JPG, PNG, GIF, WebP, BMP, TIFF, SVG
- **Vídeos**: MP4, MKV, AVI, MOV, WebM (requer libmpv)
- **PDFs**: Visualização completa com WebView2
- **GIFs**: Reprodução automática com controle de velocidade

## 📚 Documentação

### 📖 Documentos Técnicos
Acesse a pasta [`docs/`](docs/) para documentação completa:

- **[Visão Geral](docs/01_overview.md)** - Introdução e arquitetura de alto nível
- **[Build e Debug](docs/02_build_run_debug.md)** - Como compilar, executar e debugar
- **[Arquitetura](docs/03_architecture.md)** - Detalhes da arquitetura e camadas
- **[Mapa do Repositório](docs/04_module_map.md)** - Estrutura de arquivos e módulos
- **[Dependências](docs/05_dependencies_stack.md)** - Stack tecnológico completo
- **[Fluxos Principais](docs/06_key_flows.md)** - Como os principais fluxos funcionam
- **[Storage e Config](docs/07_storage_config.md)** - Onde e como dados são armazenados
- **[Logs e Erros](docs/08_logging_errors_telemetry.md)** - Sistema de logs e debugging
- **[Playbook de Suporte](docs/09_support_playbook.md)** - Guia para suporte e troubleshooting

### 🔗 Links Rápidos
- [Documentação Principal](docs/INDEX.md) - Índice completo da documentação
- [Fluxo de Navegação](docs/06_key_flows.md#1-navegação-para-pasta) - Como navegação funciona
- [Sistema de Preview](docs/06_key_flows.md#2-preview-de-arquivo) - Como preview de arquivos funciona
- [Cache e Performance](docs/07_storage_config.md#cache-de-thumbnails) - Otimizações de cache

## 🔧 Desenvolvimento

### Configuração do Ambiente
```bash
# Instalar Rust
rustup toolchain install stable
rustup default stable-msvc

# Verificar instalação
rustc --version
cargo --version
```

### Build e Testes
```bash
# Build de desenvolvimento
cargo build

# Build otimizado
cargo build --release

# Executar com logs
cargo run 2>&1 | Tee-Object "debug.log"

# Executar benchmarks
cargo bench
```

### Estrutura do Projeto
```
src/
├── app/                    # Estado e lógica principal
├── application/            # Serviços de negócio
├── domain/                 # Modelos de dados
├── infrastructure/         # Integrações com sistema
├── pdf_viewer/            # Visualizador PDF
├── tabs/                  # Sistema de abas
├── ui/                    # Interface do usuário
└── workers/               # Processamento em background
```

### Debug e Profiling
```bash
# Executar com debugger
cargo run

# Profile com flamegraph
cargo install flamegraph
cargo flamegraph --bin mtt-file-manager

# Verificar performance
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "frame_time|fps"
```

## 🔧 Solução de Problemas

### Problemas Comuns

#### Aplicativo não inicia
```powershell
# Verificar dependências
Get-Item "libmpv-2.dll" -ErrorAction SilentlyContinue
winget install Microsoft.EdgeWebView2Runtime
```

#### Performance lenta
```powershell
# Verificar uso de recursos
Get-Process mtt-file-manager | Select-Object CPU, WorkingSet

# Limpar cache se necessário
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

#### Thumbnails não aparecem
```powershell
# Capturar logs de debug
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "THUMB|ERROR" | Tee-Object "thumb_debug.log"
```

### Logs e Debugging
```powershell
# Executar com logging completo
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "full_debug.log"

# Filtrar por categoria
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "ERROR|WARN"

# Ver Event Viewer se crashou
Get-EventLog -LogName Application -Source "Application Error" | Where-Object { $_.Message -match "mtt-file-manager" }
```

### Reportando Bugs
Use o [playbook de suporte](docs/09_support_playbook.md) para reportar issues:

1. **Colete logs**: Execute com redirecionamento de stderr
2. **Documente passos**: Como reproduzir o problema
3. **Informe sistema**: Windows versão, hardware
4. **Anexe arquivos**: Logs, screenshots se relevante

## 🤝 Contribuição

### Padrões de Código
- Siga o estilo Rust padrão: `cargo fmt`
- Resolva warnings: `cargo clippy`
- Adicione testes para novas funcionalidades
- Documente APIs públicas

### Processo de Contribuição
1. Fork o repositório
2. Crie branch para sua feature (`git checkout -b feature/amazing-feature`)
3. Commit suas mudanças (`git commit -m 'Add amazing feature'`)
4. Push para branch (`git push origin feature/amazing-feature`)
5. Abra um Pull Request

### Áreas de Contribuição
- **Performance**: Otimizações de algoritmos
- **UI/UX**: Melhorias de interface
- **Features**: Novas funcionalidades
- **Bug fixes**: Correções de problemas
- **Documentação**: Melhorias nos docs

## 📄 Licença

⚠️ **Licença não especificada no código atual**

Este projeto atualmente não tem uma licença definida. Por favor, entre em contato com os mantenedores para informações sobre licenciamento.

## 🙏 Agradecimentos

- **Rust Community** - Por uma linguagem incrível
- **egui/eframe** - Framework de GUI excelente
- **windows-rs** - Bindings seguros para Windows
- **libmpv** - Player de vídeo de alta performance
- **Contribuidores** - Todos que contribuem para o projeto

## 📞 Suporte

Para suporte:
1. Consulte a [documentação](docs/) primeiro
2. Use o [playbook de suporte](docs/09_support_playbook.md)
3. Reporte issues com informações completas
4. Inclua logs e detalhes de reprodução

---

**MTT File Manager** - Um gerenciador de arquivos moderno, rápido e nativo para Windows.