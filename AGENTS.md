# Regras e Metodologia para Agentes de IA

Este documento define um conjunto de regras, comportamentos e metodologias para agentes de IA seguirem na análise e resolução de problemas de engenharia de software.

---

## ⛔ REGRA ZERO - VERIFICAÇÃO OBRIGATÓRIA ANTES DE QUALQUER AÇÃO

```
╔══════════════════════════════════════════════════════════════════════════════╗
║  ANTES DE EXECUTAR QUALQUER COMANDO, EDITAR QUALQUER ARQUIVO OU PROPOR      ║
║  QUALQUER SOLUÇÃO, O AGENTE DEVE COMPLETAR ESTE CHECKLIST:                  ║
╠══════════════════════════════════════════════════════════════════════════════╣
║                                                                              ║
║  □ 1. ENTENDI O PEDIDO?                                                     ║
║       - O que EXATAMENTE o usuário pediu?                                   ║
║       - Estou assumindo algo que não foi dito? → SE SIM, PERGUNTE          ║
║       - Há ambiguidade no pedido? → SE SIM, PERGUNTE                       ║
║                                                                              ║
║  □ 2. LI O CÓDIGO RELEVANTE?                                                ║
║       - Li os arquivos que vou modificar? → SE NÃO, LEIA PRIMEIRO          ║
║       - Entendi como funciona atualmente? → SE NÃO, INVESTIGUE             ║
║       - Sei quais são as dependências? → SE NÃO, MAPEIE                    ║
║                                                                              ║
║  □ 3. ESTOU NO ESCOPO?                                                      ║
║       - Isso foi explicitamente solicitado? → SE NÃO, NÃO FAÇA             ║
║       - Estou adicionando algo "extra"? → SE SIM, PARE E PERGUNTE          ║
║       - Isso pode quebrar algo existente? → SE SIM, ALERTE O USUÁRIO       ║
║                                                                              ║
║  □ 4. COMANDO DESTRUTIVO?                                                   ║
║       - É um comando que deleta/limpa dados? (rm, clean, reset, etc)       ║
║       - → SE SIM: NUNCA execute sem permissão EXPLÍCITA do usuário         ║
║       - → Alerte sobre consequências ANTES de executar                      ║
║  □ 5. ESSA ALTERAÇÃO IRÁ "QUEBRAR" A FUNCIONALIDADE DE OUTRAS FUNÇÕES?     ║
║                                                                            ║
║       - → SE SIM: NUNCA execute sem permissão EXPLÍCITA do usuário         ║
║       - → Alerte sobre consequências ANTES de executar                     ║
╠══════════════════════════════════════════════════════════════════════════════╣
║  ⚠️  SE QUALQUER ITEM ACIMA FALHAR → PARE E PERGUNTE AO USUÁRIO            ║
║  ⚠️  ESTAS REGRAS TÊM PRIORIDADE SOBRE QUALQUER "INSTINTO" OU OTIMIZAÇÃO   ║
║  ⚠️  VELOCIDADE NÃO JUSTIFICA PULAR VERIFICAÇÕES                           ║
╚══════════════════════════════════════════════════════════════════════════════╝
```

---

# MTT File Manager - Guia do Projeto para Agentes de IA

## Visão Geral do Projeto

O **MTT File Manager** é um gerenciador de arquivos nativo para Windows desenvolvido em Rust, com interface moderna e recursos avançados de visualização de mídia. Oferece navegação em abas, preview integrado de arquivos e integração profunda com o Windows.

### Principais Funcionalidades
- **Interface borderless customizada** - Janela moderna sem bordas tradicionais
- **Navegação em abas** - Múltiplas abas com histórico independente
- **Preview integrado** - Visualização de imagens, vídeos, GIFs e PDFs
- **Reprodução de vídeo** - Player baseado em libmpv
- **Thumbnails inteligentes** - Cache multi-nível com geração otimizada
- **Integração Windows** - Menu de contexto nativo, lixeira, OneDrive

---

## Stack Tecnológico

| Categoria | Tecnologia | Versão | Propósito |
|-----------|------------|---------|-----------|
| **Linguagem** | Rust | 2021 Edition | Core language |
| **GUI** | eframe/egui | 0.31 | Interface gráfica (immediate mode) |
| **Windows API** | windows-rs | 0.61 | Integração nativa com Windows |
| **Vídeo** | libmpv2 | 5.0.3 | Reprodução de vídeo |
| **PDF** | WebView2 | - | Visualização de PDFs |
| **Cache** | SQLite (rusqlite) | 0.32 | Persistência de thumbnails |
| **Imagens** | image crate | 0.25 | Processamento de imagens |
| **Paralelismo** | rayon | 1.10 | Processamento paralelo |

### Dependências de Runtime (Não incluídas no build)
- **libmpv-2.dll** - Necessária para reprodução de vídeo
- **Microsoft Edge WebView2 Runtime** - Necessário para visualização de PDFs

---

## Estrutura do Projeto

```
src/
├── app/                    # Estado e lógica principal
│   ├── operations/         # Handlers de operações (navegação, clipboard, etc.)
│   ├── state.rs            # Struct ImageViewerApp principal
│   ├── init.rs             # Inicialização da aplicação
│   └── ...
├── application/            # Serviços de lógica de negócios
│   ├── navigation.rs       # Histórico de navegação
│   ├── file_operations.rs  # Operações de arquivo
│   ├── clipboard.rs        # Gerenciamento de clipboard
│   └── ...
├── domain/                 # Modelos de dados e regras de negócio
│   ├── file_entry.rs       # Struct FileEntry
│   ├── errors.rs           # AppError enum e helpers
│   └── thumbnail.rs        # Modelo de thumbnail
├── infrastructure/         # Integrações com sistema
│   ├── windows/            # APIs Windows (Shell, COM, etc.)
│   ├── media/              # FFmpeg e aceleração de hardware
│   ├── disk_cache.rs       # Cache SQLite
│   └── ...
├── ui/                     # Interface do usuário
│   ├── app_impl.rs         # Implementação eframe::App
│   ├── views/              # Grid view, List view, Computer view
│   ├── components/         # Componentes reutilizáveis
│   └── ...
├── workers/                # Threads de background
│   ├── thumbnail_worker.rs # Workers de thumbnail
│   ├── file_operation_worker.rs
│   └── ...
├── tabs/                   # Sistema de abas
├── pdf_viewer/             # Visualizador PDF (WebView2)
├── main.rs                 # Entry point
└── lib.rs                  # Exports da lib
```

---

## Comandos de Build e Teste

### Build
```bash
# Build de desenvolvimento
cargo build

# Build de produção (otimizado)
cargo build --release

# Build sem features opcionais
cargo build --no-default-features
```

### Execução
```bash
# Executar em modo debug
cargo run

# Executar release
cargo run --release

# Executar com logs (PowerShell)
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "debug.log"
```

### Testes
```bash
# Todos os testes
cargo test

# Testes com output
cargo test -- --nocapture

# Benchmarks
cargo bench
```

### Lint e Formatação
```bash
# Formatar código
cargo fmt

# Executar clippy
cargo clippy
```

---

## Arquitetura

O projeto segue uma arquitetura em camadas:

```
┌─────────────────────────────────────────────┐
│              UI Layer (src/ui/)             │
│    (eframe/egui - Immediate Mode GUI)       │
├─────────────────────────────────────────────┤
│         Application Layer (src/app/)        │
│    (Estado global, coordenação, handlers)   │
├─────────────────────────────────────────────┤
│       Business Logic (src/application/)     │
│    (Navigation, File Operations, Sorting)   │
├─────────────────────────────────────────────┤
│         Domain Layer (src/domain/)          │
│    (FileEntry, AppError, enums puros)       │
├─────────────────────────────────────────────┤
│    Infrastructure Layer (src/infrastructure/)│
│    (Windows API, SQLite, Workers)           │
└─────────────────────────────────────────────┘
```

### Comunicação entre Camadas
- **UI ↔ Workers**: Canais MPSC (crossbeam-channel)
- **Estado Compartilhado**: `Arc<Mutex<T>>` e `Arc<DashMap<K,V>>`
- **Cache Multi-nível**: Memória (LRU) → Disco (SQLite) → GPU (textures)

---

## Tipos Principais

### AppError (src/domain/errors.rs)
```rust
pub enum AppError {
    Security(SecurityError),
    WindowsApi(String),
    Io(std::io::Error),
    ThumbnailExtraction { path: PathBuf, source: anyhow::Error },
    FileOperation(String),
    InvalidState(String),
    Config(String),
    Worker(String),
    UiRendering(String),
}
```

### FileEntry (src/domain/file_entry.rs)
```rust
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub folder_cover: Option<PathBuf>,
    pub drive_info: Option<DriveInfo>,
    pub sync_status: SyncStatus,
    pub deletion_date: Option<String>,
    pub recycle_original_path: Option<PathBuf>,
}
```

### Enums Importantes
- `SortMode { Name, Date, Size, Type, DriveTotalSpace, DriveFreeSpace }`
- `ViewMode { Grid, List }`
- `SyncStatus { None, CloudOnly, Syncing, Pinned, LocallyAvailable }`

---

## Convenções de Código

### Estilo
- Siga o estilo Rust padrão (`cargo fmt`)
- Resolva todos os warnings do `cargo clippy`
- Use nomes descritivos em português para variáveis quando o contexto for do domínio

### Logging
```rust
// Use eprintln! com categoria prefixada
eprintln!("[INIT] Starting application...");
eprintln!("[CACHE] Cache hit for: {:?}", path);
eprintln!("[ERROR] Failed to load: {}", error);
eprintln!("[WARN] Falling back to default");
eprintln!("[PERF] Frame time: {}ms", frame_ms);
```

### Tratamento de Erros
```rust
// Use AppError e helpers
use crate::domain::errors::{windows_error, file_operation_error, AppResult};

// Em vez de unwrap/expect em código de produção
let result = operation.map_err(|e| windows_error(&format!("context: {}", e)))?;

// Macros disponíveis
let value = safe_unwrap!(result, "context");
let value = safe_expect!(option, "expected message");
```

### Documentação
- Documente APIs públicas com doc comments (`///`)
- Explique o POR QUÊ, não apenas o O QUÊ
- Referencie arquivos relevantes em explicações

---

## Localização de Dados

### Diretório de Cache
```
%LOCALAPPDATA%\MTT-File-Manager\
├── thumbnails\
│   ├── thumbnails.db       # SQLite com metadados
│   └── *.webp              # Arquivos de thumbnail
└── virtual_drive_config.json
```

### Para Limpar Cache
```powershell
Remove-Item "$env:LOCALAPPDATA\MTT-File-Manager" -Recurse -Force
```

---

## Debug e Troubleshooting

### Capturar Logs
```powershell
# Método completo com script
.\run_with_logs.ps1

# Redirecionamento manual
.\target\release\mtt-file-manager.exe 2>&1 | Tee-Object "app.log"

# Filtrar por categoria
.\target\release\mtt-file-manager.exe 2>&1 | Select-String "ERROR|WARN"
```

### Variáveis de Ambiente Úteis
```powershell
$env:RUST_BACKTRACE=1           # Backtrace em panics
$env:RUST_LOG="debug"           # Logging detalhado (se implementado)
```

### Problemas Comuns

| Problema | Solução |
|----------|---------|
| "libmpv-2.dll not found" | Copiar DLL para pasta do executável |
| "WebView2 not available" | `winget install Microsoft.EdgeWebView2Runtime` |
| Thumbnails não aparecem | Verificar logs com `Select-String "THUMB\|ERROR"` |
| Performance lenta | Verificar `frame_time` nos logs, limpar cache |

---

## Considerações de Segurança

- **Path Traversal**: Sempre valide paths com `std::path`
- **Symbolic Links**: Tratados com cuidado em operações recursivas
- **COM Security**: Inicialização apropriada em `infrastructure/windows/`
- **Validação de Input**: Verifique bounds em índices e tamanhos
- **Execução de Comandos**: NUNCA execute comandos shell com input do usuário sem sanitização

---

## Restrições Importantes

### Windows Only
Este projeto é **Windows-only** devido ao uso extensivo de Windows APIs. Não tente compilar para outras plataformas.

### Features do Cargo
- `notify-watcher` (padrão) - Habilita monitoramento de filesystem via notify crate
- Drive Watcher (sempre ativo) - Monitoramento otimizado via ReadDirectoryChangesW (Windows nativo)
- Para build sem watcher: `cargo build --no-default-features`

### File System Watching
O projeto usa duas implementações de file watching:

1. **Drive Watcher (File Pilot optimization)** - Sistema primário
   - Usa `ReadDirectoryChangesW` diretamente no drive raiz (ex: `C:\`)
   - Monitora drive inteiro, filtra eventos por pasta atual
   - Zero overhead na navegação (não recria watchers)
   - Arquivos: `src/infrastructure/drive_watcher.rs`, `drive_watcher_integration.rs`
   - Documentação: `docs/10_file_pilot_optimizations.md`

2. **Notify Watcher** - Sistema legacy (fallback)
   - Usa crate `notify` para monitorar pasta individual
   - Recria watcher a cada navegação (overhead)
   - Mantido como fallback para UNC paths (`\\server\share`)
   - Arquivo: `src/infrastructure/watcher.rs`

### Seleção Automática
```rust
if is_local_drive(path) {
    use drive_watcher;  // Preferido
} else {
    use notify_watcher; // UNC paths
}
```

### Profile de Release
```toml
[profile.release]
opt-level = 3      # Otimização máxima
lto = true         # Link Time Optimization
codegen-units = 1  # Single codegen unit
```

---

## Documentação Adicional

A pasta `docs/` contém documentação técnica completa:

- **[docs/INDEX.md](docs/INDEX.md)** - Índice da documentação
- **[docs/01_overview.md](docs/01_overview.md)** - Visão geral
- **[docs/02_build_run_debug.md](docs/02_build_run_debug.md)** - Build e debug
- **[docs/03_architecture.md](docs/03_architecture.md)** - Arquitetura detalhada
- **[docs/04_module_map.md](docs/04_module_map.md)** - Mapa de módulos
- **[docs/05_dependencies_stack.md](docs/05_dependencies_stack.md)** - Stack de dependências
- **[docs/06_key_flows.md](docs/06_key_flows.md)** - Fluxos principais
- **[docs/07_storage_config.md](docs/07_storage_config.md)** - Storage e config
- **[docs/08_logging_errors_telemetry.md](docs/08_logging_errors_telemetry.md)** - Logs
- **[docs/09_support_playbook.md](docs/09_support_playbook.md)** - Playbook de suporte

---

## Princípios Fundamentais (Resumo)

### 1. Precisão Técnica Acima de Validação
- Forneça informações técnicas objetivas
- Discorde quando necessário
- Investigue para encontrar a verdade antes de confirmar suposições

### 2. Comunicação Direta e Concisa
- Respostas curtas e focadas
- Evite superlativos desnecessários
- Use markdown para formatação

### 3. Minimalismo - Evite Over-Engineering
- Apenas mudanças diretamente solicitadas
- Soluções simples e focadas
- Não adicione features não solicitadas
- Não refatore código além do necessário

### 4. Perguntas Antes de Suposições
- Quando requisitos são ambíguos, PERGUNTE
- Quando há múltiplas abordagens, APRESENTE AS OPÇÕES
- Não assuma - valide entendimento

### 5. Limpeza de Código
- Se algo não é usado, DELETE completamente
- Não mantenha código morto "por precaução"
- Siga a Regra dos Três: três linhas similares são melhores que uma abstração prematura

---

*Documentação gerada para agentes de IA. Mantenha atualizada conforme o projeto evolui.*
*Última atualização: 2026-02-02*
