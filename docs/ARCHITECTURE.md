# 🏛️ Arquitetura do MTT File Manager

**Última Atualização**: Janeiro 2026

---

## Visão Geral

O MTT File Manager é um gerenciador de arquivos nativo para Windows, construído com uma arquitetura híbrida que combina:

- **Rust + egui** para a interface principal
- **MPV (libmpv2)** para reprodução de vídeo com aceleração de hardware
- **Windows APIs** para integração nativa com Shell, COM e Media Foundation

---

## Diagrama de Camadas

```
┌─────────────────────────────────────────────────────────────────┐
│                           UI Layer                               │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   egui      │  │   views/    │  │   components/           │  │
│  │   Panels    │  │  grid_view  │  │  mpv_preview.rs         │  │
│  │   Sidebar   │  │  list_view  │  │  (MPV Video)            │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│                      Application Layer                           │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │   app/operations/   (19 módulos)                        │    │
│  │   clipboard_ops, navigation, tabs, thumbnails, ...      │    │
│  └─────────────────────────────────────────────────────────┘    │
├─────────────────────────────────────────────────────────────────┤
│                     Infrastructure Layer                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
│  │   cache     │  │   windows/  │  │   workers/              │  │
│  │   SQLite    │  │  icons.rs   │  │  thumbnail_loader.rs    │  │
│  │   LRU       │  │  metadata/  │  │  folder_scanner.rs      │  │
│  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
├─────────────────────────────────────────────────────────────────┤
│                       Domain Layer                               │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │   FileEntry, ThumbnailData, MediaMetadata, errors       │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

---

## Arquitetura Híbrida: egui + MPV

### O Problema

O `egui` é um framework de UI imediata (immediate mode GUI) excelente para interfaces responsivas, mas **não suporta**:

- Decodificação de vídeo com aceleração GPU
- Codecs modernos (H.264, HEVC, VP9, AV1)
- Renderização de vídeo em textura de forma eficiente

### A Solução

Utilizamos o crate `libmpv2` para embedar uma janela MPV como child window via handle `wid`:

```
┌─────────────────────────────────────────────┐
│            Janela Principal (egui)          │
│  ┌───────────────────┬───────────────────┐  │
│  │                   │   Preview Panel   │  │
│  │   File List       │  ┌─────────────┐  │  │
│  │                   │  │    MPV     │  │  │
│  │   (egui)          │  │  (child)    │  │  │
│  │                   │  │             │  │  │
│  │                   │  └─────────────┘  │  │
│  │                   │  [Controls egui]  │  │
│  └───────────────────┴───────────────────┘  │
└─────────────────────────────────────────────┘
```

### Fluxo de Reprodução de Vídeo

- MPV é inicializado sob demanda.
- Um HWND filho é criado e passado via `wid`.
- O arquivo é carregado diretamente pelo MPV.
- O estado do player é consultado por propriedades (`time-pos`, `duration`, `pause`, `volume`).

---

## Módulos Principais

### `app/operations/` — Lógica de Aplicação

| Módulo | Responsabilidade |
|--------|------------------|
| `clipboard_ops.rs` | Copy/Cut/Paste via CF_HDROP |
| `context_menu.rs` | Menu de contexto nativo |
| `file_ops.rs` | Operações de arquivo (delete, rename) |
| `folder_loading.rs` | Carregamento assíncrono de pastas |
| `navigation.rs` | Histórico back/forward |
| `tabs.rs` | Gerenciamento de abas |
| `thumbnails.rs` | Cache e carregamento de miniaturas |

### `infrastructure/windows/` — Integração Nativa

| Módulo | APIs Windows Utilizadas |
|--------|-------------------------|
| `icons.rs` | `IShellItemImageFactory`, `SHGetFileInfoW` |
| `metadata/video.rs` | `IPropertyStore`, Media Foundation |
| `native_menu.rs` | `IContextMenu`, `TrackPopupMenu` |
| `recycle_bin.rs` | `SHFileOperationW` |
| `shell_operations.rs` | `IFileOperation` |

### `workers/` — Background Threads

| Worker | Função |
|--------|--------|
| `thumbnail_loader.rs` | Carrega thumbnails via WIC |
| `folder_scanner.rs` | Lista diretórios em background |
| `folder_preview_worker.rs` | Gera previews de pastas |

---

## Performance

### Operações Assíncronas

Todas as operações de I/O são executadas fora da thread principal:

- File scanning: `mpsc::channel` + thread
- Thumbnails: Worker pool com LRU cache
- Metadata: Lazy loading no selection

### Cache Strategy

```
┌──────────────────┐
│   LRU Cache      │  ←  Texturas em memória (256 itens)
├──────────────────┤
│   SQLite Cache   │  ←  Thumbnails WebP em disco
├──────────────────┤
│   Windows Cache  │  ←  Shell thumbnail cache
└──────────────────┘
```

---

## Requisitos de Sistema

- Windows 10/11 (64-bit)
- MPV runtime (`mpv-1.dll` ao lado do executável ou no PATH)
- ~50MB RAM base + ~2MB por aba

---

*Mantido pela equipe MTT File Manager*
