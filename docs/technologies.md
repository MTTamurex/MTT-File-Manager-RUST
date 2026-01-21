# Tecnologias Utilizadas - MTT File Manager

## 1. Linguagem de Programação

### Rust (Edition 2021)
- **Uso:** Toda a base de código
- **Criticidade:** **ALTA** - Linguagem principal do projeto
- **Versão:** Edition 2021 (definido em `Cargo.toml`)

---

## 2. Framework GUI

### eframe/egui v0.31
- **Uso:** `main.rs`, `ui/app_impl.rs`, todos os componentes de UI
- **Para que serve:** Framework de GUI imediata multiplataforma
- **Criticidade:** **ALTA** - Base de toda a interface gráfica
- **Features:** `persistence` habilitada

---

## 3. Dependências de Sistema Operacional

### windows v0.61.0
- **Uso:** `infrastructure/windows/*`
- **Para que serve:** Bindings nativos da WinAPI
- **Criticidade:** **ALTA** - Essencial para todas as operações Windows
- **Features habilitadas:**
  - `Win32_UI_Shell` - Shell API (menus, thumbnails)
  - `Win32_Graphics_Gdi` - Operações GDI (bitmaps, ícones)
  - `Win32_Media_MediaFoundation` - Extração de frames de vídeo
  - `Win32_System_Com` - COM para shell e mídia
  - `Win32_Foundation`, `Win32_Storage_FileSystem`, `Win32_UI_WindowsAndMessaging`
  - E outras 15+ features

---

## 4. Mídia e Vídeo

### libmpv2 v5.0.3 (via crate `mpv`)
- **Uso:** `ui/components/mpv_preview.rs`
- **Para que serve:** Player de vídeo embutido via libmpv
- **Criticidade:** **ALTA** - Reprodução de vídeo no painel de preview
- **Dependência externa:** Requer `mpv.lib` (presente na raiz do projeto)

### image v0.25
- **Uso:** `workers/thumbnail_worker.rs`, `main.rs`
- **Para que serve:** Decodificação de imagens (JPEG, PNG, WebP, GIF)
- **Criticidade:** **ALTA** - Pipeline de thumbnails
- **Features:** `webp`, `gif`

### webp v0.3
- **Uso:** `infrastructure/disk_cache.rs`
- **Para que serve:** Compressão lossy de thumbnails em cache
- **Criticidade:** **MÉDIA** - Otimização de cache

### kamadak-exif v0.5
- **Uso:** `infrastructure/windows/metadata/image.rs`
- **Para que serve:** Leitura de metadados EXIF de JPEGs
- **Criticidade:** **MÉDIA** - Exibição de metadados de fotos

---

## 5. SVG e Vetores

### resvg v0.44, usvg v0.44, tiny-skia v0.11
- **Uso:** `ui/svg_icons.rs`
- **Para que serve:** Renderização de ícones SVG em tempo de execução
- **Criticidade:** **ALTA** - Todos os ícones da toolbar e UI

---

## 6. Cache e Performance

### lru v0.12
- **Uso:** `app/state.rs`, `ui/cache.rs`, `ui/components/gif_manager.rs`
- **Para que serve:** Cache LRU para texturas e thumbnails em memória
- **Criticidade:** **ALTA** - Performance de scroll e navegação

### rusqlite v0.32
- **Uso:** `infrastructure/disk_cache.rs`
- **Para que serve:** Cache persistente de thumbnails em disco (SQLite)
- **Criticidade:** **MÉDIA** - Persistência entre sessões
- **Features:** `bundled` (SQLite embutido no executável)

### dashmap v5.5
- **Uso:** `infrastructure/windows/codec_registry.rs`, outros caches
- **Para que serve:** HashMap thread-safe para caches compartilhados
- **Criticidade:** **ALTA** - Caches multi-thread

---

## 7. Concorrência e Threading

### rayon v1.10
- **Uso:** `workers/thumbnail_worker.rs`
- **Para que serve:** Paralelismo data-parallel para workers
- **Criticidade:** **ALTA** - Extração paralela de thumbnails

---

## 8. Sistema de Arquivos

### walkdir v2.5
- **Uso:** `app/operations/folder_loading.rs`
- **Para que serve:** Varredura recursiva de diretórios
- **Criticidade:** **ALTA** - Listagem de pastas

### notify v6.1.1
- **Uso:** `application/watcher.rs`, `app/operations/watcher.rs`
- **Para que serve:** Monitoramento de mudanças no filesystem
- **Criticidade:** **MÉDIA** - Auto-refresh de pastas

### dirs v5.0
- **Uso:** `infrastructure/disk_cache.rs`
- **Para que serve:** Caminhos de sistema (AppData, etc.)
- **Criticidade:** **BAIXA** - Localização de cache

---

## 9. Diálogos e Clipboard

### rfd v0.15
- **Uso:** `application/navigation.rs`
- **Para que serve:** Diálogos nativos de seleção de pasta/arquivo
- **Criticidade:** **BAIXA** - "Open Folder" dialog

### clipboard-win v5.4
- **Uso:** `infrastructure/windows_clipboard.rs`
- **Para que serve:** Clipboard CF_HDROP (copiar/colar arquivos)
- **Criticidade:** **ALTA** - Operações de clipboard

---

## 10. Ordenação e Parsing

### natord v1.0
- **Uso:** `application/sorting.rs`
- **Para que serve:** Ordenação natural (file1, file2, file10...)
- **Criticidade:** **BAIXA** - UX de ordenação

### serde_json v1.0
- **Uso:** `ui/components/mpv_preview.rs`
- **Para que serve:** Parsing de respostas JSON do mpv
- **Criticidade:** **BAIXA** - Comunicação com mpv

---

## 11. Tratamento de Erros

### thiserror v2.0
- **Uso:** `domain/errors.rs`
- **Para que serve:** Derivação de tipos de erro
- **Criticidade:** **BAIXA** - Ergonomia de código

### tempfile v3.10
- **Uso:** Operações temporárias de arquivo
- **Para que serve:** Criação de arquivos temporários seguros
- **Criticidade:** **BAIXA**

---

## 12. WebView (PDF Viewer)

### WebView2 (via windows crate)
- **Uso:** `pdf_viewer/webview.rs`
- **Para que serve:** Renderização de PDF via Edge WebView2
- **Criticidade:** **MÉDIA** - Visualizador de PDF externo
- **Dependência externa:** Microsoft Edge WebView2 Runtime

---

## 13. Handle de Janela

### raw-window-handle v0.6
- **Uso:** `ui/components/mpv_preview.rs`
- **Para que serve:** Interoperabilidade de handles de janela
- **Criticidade:** **MÉDIA** - Embedding de mpv em janela eframe

---

## 14. Build e Empacotamento

### winresource v0.1
- **Uso:** `build.rs`
- **Para que serve:** Embed de ícone e metadados no executável Windows
- **Criticidade:** **BAIXA** - Branding do executável

---

## 15. Dependências Comentadas (Não Utilizadas)

```rust
// video-rs = { version = "0.9", features = ["ndarray"] }
// ndarray = "0.16"
// rodio = "0.19"
```
**Status:** Código morto - provavelmente experimentos anteriores

---

## Sumário de Criticidade

| Nível | Tecnologias |
|-------|-------------|
| **ALTA** | Rust, eframe, windows crate, libmpv2, image, resvg, lru, dashmap, rayon, walkdir, clipboard-win |
| **MÉDIA** | rusqlite, webp, kamadak-exif, notify, WebView2, raw-window-handle |
| **BAIXA** | rfd, dirs, natord, serde_json, thiserror, tempfile, winresource |
