# Estrutura do Projeto - MTT File Manager

## Visão Geral

```
MTT-File-Manager-RUST/
├── .agent/                    # Configurações de agente (Claude/IDE)
├── .cargo/                    # Configuração local do Cargo
├── .git/                      # Repositório Git
├── .github/                   # GitHub workflows/actions
├── .vscode/                   # Configurações VS Code
├── assets/                    # Assets compilados no executável
├── docs/                      # Documentação técnica
├── src/                       # Código-fonte Rust
├── target/                    # Artefatos de build (ignorado no git)
├── Cargo.toml                 # Manifest do projeto
├── Cargo.lock                 # Lockfile de dependências
├── build.rs                   # Build script (embed ícone)
├── appicon.ico                # Ícone Windows (182KB)
├── appicon.png                # Ícone PNG (343KB)
├── mpv.lib                    # Biblioteca estática mpv (173KB)
└── [scripts auxiliares]       # PowerShell scripts de teste
```

---

## Diretório Raiz

| Arquivo/Pasta | Função | Importância |
|---------------|--------|-------------|
| `Cargo.toml` | Manifest: nome, versão, dependências | **CRÍTICO** |
| `Cargo.lock` | Versões exatas de dependências | **ALTO** |
| `build.rs` | Embed de ícone e metadados Windows | MÉDIO |
| `appicon.ico` | Ícone do executável Windows | BAIXO |
| `appicon.png` | Ícone de alta resolução | BAIXO |
| `mpv.lib` | Linker library para libmpv | **CRÍTICO** para vídeo |
| `run_with_logs.ps1` | Script de teste com captura de logs | DEBUG |
| `test_standalone.ps1` | Teste de executável portátil | DEBUG |
| `errors.txt` | Log de erros (debug) | TEMP |
| `hd_perf_check.txt` | Notas de performance | TEMP |
| `temp_snippet_shell_ops.rs` | Código temporário | **CÓDIGO MORTO** |
| `MTT-File-Manager-RUST.code-workspace` | Workspace VS Code | IDE |
| `.gitignore` | Regras de exclusão Git | CONFIG |

---

## Diretório `assets/`

**Função:** Assets embarcados no executável via `include_bytes!()`

```
assets/
├── icons/                     # 28 ícones SVG
│   ├── copy.svg, cut.svg, delete.svg, paste.svg
│   ├── nav_back.svg, nav_forward.svg, nav_up.svg
│   ├── folder.svg, folder_new.svg, drive.svg
│   ├── view_grid.svg, view_list.svg
│   ├── play.svg, pause.svg, vol_high.svg, vol_mute.svg
│   ├── search.svg, refresh.svg, rename.svg
│   ├── maximize.svg, minimize.svg, minimize_2.svg
│   ├── home.svg, info.svg, properties.svg
│   ├── external-link.svg, headphones.svg, languages.svg
│   └── [total: 28 arquivos SVG]
└── remixicon.ttf              # Fonte de ícones Remix (603KB)
```

**Relação:** Consumidos por `src/embedded_assets.rs` e `src/ui/svg_icons.rs`

---

## Diretório `src/`

**Função:** Todo o código-fonte Rust

### Arquivos Raiz

| Arquivo | Linhas | Função |
|---------|--------|--------|
| `main.rs` | 129 | Entry point, configuração de janela e fontes |
| `lib.rs` | 15 | Declaração de módulos públicos |
| `embedded_assets.rs` | 74 | Constantes com assets embutidos |

---

### Diretório `src/app/`

**Função:** Estado central da aplicação e operações

```
app/
├── mod.rs                     # Exports do módulo (520B)
├── state.rs                   # ImageViewerApp struct (9KB, 221 linhas)
├── init.rs                    # ImageViewerApp::new() (17KB, 430 linhas)
└── operations/                # 19 módulos de operações
    ├── mod.rs                 # Exports (1.4KB)
    ├── clipboard_ops.rs       # Cópia via clipboard (4.5KB)
    ├── context_menu.rs        # Handler de menu de contexto (15KB)
    ├── file_ops.rs            # Delete, criar pasta (5.2KB)
    ├── folder_loading.rs      # Carregamento de diretórios (13KB)
    ├── icons.rs               # Helpers de ícones (1KB)
    ├── message_handler.rs     # Processamento de msgs async (17KB)
    ├── metadata.rs            # Tratamento de metadados (3.3KB)
    ├── navigation.rs          # Back/Forward/Up (7KB)
    ├── preferences.rs         # Preferências de usuário (2.6KB)
    ├── recycle_bin_ops.rs     # Operações de lixeira (4.5KB)
    ├── selection.rs           # Lógica de seleção (6.8KB)
    ├── tabs.rs                # Operações de tabs (2.7KB)
    ├── thumbnails.rs          # Requisições de thumbnails (1KB)
    ├── trait_impls.rs         # Implementações de traits (1.8KB)
    ├── ui_rendering.rs        # Orquestração de render (38KB) ⚠️ GRANDE
    ├── view_setup.rs          # Configuração de views (8KB)
    ├── watcher.rs             # File system watcher (1.7KB)
    └── window.rs              # Operações de janela (2.3KB)
```

---

### Diretório `src/application/`

**Função:** Serviços de lógica de negócio

```
application/
├── mod.rs                     # Exports (543B)
├── clipboard.rs               # Serviço de clipboard (4.3KB)
├── context_menu.rs            # Actions de menu (7.4KB)
├── file_operations.rs         # Operações de arquivo (5.3KB)
├── navigation.rs              # NavigationHistory (3.1KB)
├── notification.rs            # Sistema de notificações (4.4KB)
├── renaming.rs                # Lógica de rename (1KB)
├── sorting.rs                 # Algoritmos de ordenação (6.6KB)
├── state.rs                   # Estado compartilhado (9.8KB)
└── watcher.rs                 # Configuração de watcher (1.3KB)
```

---

### Diretório `src/domain/`

**Função:** Modelos de dados e tipos

```
domain/
├── mod.rs                     # Exports (79B)
├── file_entry.rs              # FileEntry, DriveInfo, enums (4.2KB)
├── errors.rs                  # AppError, macros (5KB)
└── thumbnail.rs               # ThumbnailData (266B)
```

---

### Diretório `src/infrastructure/`

**Função:** Integração com sistema operacional

```
infrastructure/
├── mod.rs                     # Exports (213B)
├── cache.rs                   # Placeholder (45B) ⚠️ VAZIO
├── disk_cache.rs              # ThumbnailDiskCache SQLite (17KB)
├── onedrive.rs                # Detecção OneDrive (5.7KB)
├── security.rs                # Validação de caminhos (12KB)
├── watcher.rs                 # Placeholder (44B) ⚠️ VAZIO
├── windows_clipboard.rs       # CF_HDROP clipboard (6.7KB)
├── media/                     # Extração de mídia
│   ├── mod.rs                 # Exports (57B)
│   ├── ffmpeg_session.rs      # Sessão FFmpeg (10KB)
│   ├── hardware_acceleration.rs # HW accel detection (11KB)
│   └── tests_hw.rs            # Testes de HW accel (566B)
└── windows/                   # Windows API wrappers
    ├── mod.rs                 # Exports (1.1KB)
    ├── bitmap_conversion.rs   # HBITMAP → RGBA (4.9KB)
    ├── codec_registry.rs      # Cache de nomes de codecs (22KB)
    ├── device_change.rs       # Detecção de dispositivos (4.2KB)
    ├── drives.rs              # Listagem de drives (5.6KB)
    ├── file_system.rs         # Helpers de filesystem (934B)
    ├── file_type.rs           # Tipos de arquivo (6.5KB)
    ├── formatting.rs          # Formatação de tamanhos (3KB)
    ├── icons.rs               # Extração de ícones shell (21KB)
    ├── media_foundation.rs    # MF frame extraction (16KB)
    ├── native_menu.rs         # Shell menu extraction (14KB)
    ├── recycle_bin.rs         # IShellItem2 Recycle Bin (21KB)
    ├── shell_folder.rs        # IShellFolder navegação (4.9KB)
    ├── shell_operations.rs    # SHFileOperation wrappers (12KB)
    ├── system_info.rs         # Informações de sistema (3KB)
    ├── window_subclass.rs     # Borderless window (14KB)
    └── metadata/              # Extração de metadados
        ├── mod.rs             # Exports (2.2KB)
        ├── audio_sniffing.rs  # Audio stream info (9KB)
        ├── image.rs           # EXIF metadata (8.4KB)
        ├── property_keys.rs   # PROPERTYKEY defs (7.6KB)
        ├── utils.rs           # Helpers de metadata (8.4KB)
        ├── video.rs           # Video metadata (14KB)
        └── video_sniffing.rs  # Video stream info (9.2KB)
```

---

### Diretório `src/ui/`

**Função:** Interface gráfica e componentes

```
ui/
├── mod.rs                     # Exports (414B)
├── app_impl.rs                # eframe::App impl (13KB)
├── cache.rs                   # TextureCache in-memory (13KB)
├── context_menu.rs            # Menu UI customizado (22KB)
├── icon_loader.rs             # Carregamento de ícones (11KB)
├── navigation.rs              # Breadcrumb/path bar (5.7KB)
├── preview_panel.rs           # Painel lateral preview (59KB) ⚠️ MUITO GRANDE
├── sidebar.rs                 # Sidebar com pastas (12KB)
├── status_bar.rs              # Barra de status (6KB)
├── svg_icons.rs               # Renderização SVG (6.5KB)
├── tab_bar.rs                 # Barra de abas (18KB)
├── theme.rs                   # Definições de tema (2.4KB)
├── toolbar.rs                 # Barra de ferramentas (18KB)
├── widgets.rs                 # Widgets customizados (5.5KB)
├── app/                       # Sub-componentes de app
│   ├── mod.rs                 # Exports (217B)
│   ├── input.rs               # Handling de input (6KB)
│   ├── lifecycle.rs           # Lifecycle hooks (3.4KB)
│   ├── menu_handler.rs        # Handlers de menu (6KB)
│   ├── notifications.rs       # UI de notificações (2.3KB)
│   └── panels.rs              # Layout de painéis (18KB)
├── components/                # Componentes reutilizáveis
│   ├── mod.rs                 # Exports (260B)
│   ├── gif_manager.rs         # Gerenciador de GIFs (7.4KB)
│   ├── item_slot.rs           # Célula de arquivo (27KB)
│   ├── media_preview.rs       # Preview de imagens (13KB)
│   ├── mpv_preview.rs         # Player mpv (20KB)
│   └── video_menu.rs          # Menu de vídeo (12KB)
└── views/                     # Views de listagem
    ├── mod.rs                 # Exports (308B)
    ├── common.rs              # Helpers comuns (1.2KB)
    ├── computer_view.rs       # View "Este PC" (5KB)
    ├── grid_view.rs           # Grid de arquivos (23KB)
    └── list_view.rs           # Lista detalhada (34KB)
```

---

### Diretório `src/workers/`

**Função:** Threads de background

```
workers/
├── mod.rs                           # Exports (247B)
├── thumbnail_worker.rs              # Pipeline de thumbnails (33KB, 925 linhas) ⚠️
├── batch_thumbnail_loader.rs        # Carregamento em lote (6.9KB)
├── file_operation_worker.rs         # File ops async (5KB)
├── folder_preview_worker.rs         # Preview de pastas (2.9KB)
├── folder_scanner.rs                # Placeholder (47B) ⚠️ VAZIO
└── thumbnail_loader.rs              # Placeholder (49B) ⚠️ VAZIO
```

---

### Diretório `src/tabs/`

**Função:** Sistema de abas

```
tabs/
└── mod.rs                     # TabManager, TabState (11KB, 345 linhas)
```

---

### Diretório `src/pdf_viewer/`

**Função:** Visualizador de PDF/Imagem externo

```
pdf_viewer/
├── mod.rs                     # Entry points (3.6KB)
├── thread.rs                  # Thread STA dedicada (841B)
├── webview.rs                 # WebView2 integration (17KB)
└── window.rs                  # Janela do viewer (2.8KB)
```

---

## Arquivos Notáveis

### Arquivos Muito Grandes (>15KB)
| Arquivo | Tamanho | Observação |
|---------|---------|------------|
| `ui/preview_panel.rs` | 59KB | Candidato a refatoração |
| `workers/thumbnail_worker.rs` | 33KB | Pipeline complexo, justificado |
| `app/operations/ui_rendering.rs` | 38KB | Orquestrador central |
| `ui/components/item_slot.rs` | 27KB | Muita lógica de renderização |

### Arquivos Vazios/Placeholder
| Arquivo | Status |
|---------|--------|
| `infrastructure/cache.rs` | 45 bytes - apenas comentário |
| `infrastructure/watcher.rs` | 44 bytes - apenas comentário |
| `workers/folder_scanner.rs` | 47 bytes - stub |
| `workers/thumbnail_loader.rs` | 49 bytes - stub |

### Arquivos Temporários (Raiz)
| Arquivo | Status |
|---------|--------|
| `temp_snippet_shell_ops.rs` | **CÓDIGO MORTO** - deve ser removido |
| `errors.txt` | Log de debug |
| `hd_perf_check.txt` | Notas de performance |
