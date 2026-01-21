# Configuração e Ambiente - MTT File Manager

## 1. Arquivos de Configuração

### `Cargo.toml`
- **Localização:** Raiz do projeto
- **Função:** Manifest do projeto Rust
- **Conteúdo:**
  - Nome: `mtt-file-manager`
  - Versão: `0.1.0`
  - Edition: `2021`
  - Dependências (ver `technologies.md`)
  - Profile de release otimizado (`opt-level=3`, `lto=true`)

### `build.rs`
- **Função:** Script de build-time
- **Ações:**
  - Embute `appicon.ico` no executável Windows
  - Define metadados: FileDescription, ProductName, CompanyName

### `.gitignore`
- **Função:** Exclusões de versionamento
- **Exclusões principais:**
  - `/target/` - Artefatos de build
  - `Cargo.lock` - ⚠️ **NOTA:** Deveria ser versionado para executáveis
  - `.vscode/`, `.idea/` - Configs de IDE
  - `*.log`, `errors.txt` - Logs de debug

---

## 2. Variáveis de Ambiente

### Variáveis Utilizadas pelo Código

| Variável | Uso | Obrigatória |
|----------|-----|-------------|
| `CARGO_MANIFEST_DIR` | `build.rs` - Localização do projeto | Auto (Cargo) |
| `APPDATA` | Cache de thumbnails via crate `dirs` | Auto (Windows) |

### Variáveis Implícitas (Sistema)

O projeto depende de caminhos hardcoded do Windows:
```rust
// Em main.rs - Fontes do sistema
"C:\\Windows\\Fonts\\segoeui.ttf"
"C:\\Windows\\Fonts\\seguisym.ttf"
"C:\\Windows\\Fonts\\ARIALUNI.TTF"
```

---

## 3. Dependências de Sistema Operacional

### Windows (Obrigatório)

O projeto é **exclusivamente Windows** devido a:
- Windows API (`windows` crate)
- Shell operations (IShellItem, IContextMenu)
- Media Foundation
- COM infrastructure

### Versão Mínima do Windows

⚠️ **Indeterminado com base no código atual**

Provável requisito: **Windows 10** ou superior devido a:
- WebView2 (Edge Chromium)
- APIs modernas de Media Foundation

---

## 4. Dependências Externas (Runtime)

### libmpv / mpv

**Arquivo:** `mpv.lib` (173KB na raiz)

**Requisito runtime:**
- `libmpv-2.dll` deve estar no PATH ou junto ao executável
- Ou mpv instalado no sistema

**Configuração mpv** (definida em `mpv_preview.rs`):
```rust
mpv.set_property("vo", "gpu")?;
mpv.set_property("gpu-api", "d3d11")?;
mpv.set_property("hwdec", "d3d11va")?;
mpv.set_property("keep-open", "yes")?;
mpv.set_property("idle", "yes")?;
```

### Microsoft Edge WebView2

**Uso:** PDF Viewer

**Requisito:** Microsoft Edge WebView2 Runtime instalado
- Geralmente presente em Windows 10 21H2+ e Windows 11
- Pode ser baixado: https://developer.microsoft.com/microsoft-edge/webview2/

---

## 5. Flags e Modos de Execução

### Modos de Build

| Modo | Comando | Uso |
|------|---------|-----|
| Debug | `cargo build` | Desenvolvimento |
| Release | `cargo build --release` | Produção |

### Flags de Compilação

Definidas em `Cargo.toml` para release:
```toml
[profile.release]
opt-level = 3      # Otimização máxima
lto = true         # Link-Time Optimization
codegen-units = 1  # Single codegen unit (mais lento, menor binário)
```

---

## 6. Configurações Embarcadas

### Constantes no Código

| Constante | Arquivo | Valor |
|-----------|---------|-------|
| `PATH_PADRAO` | `app/init.rs` | `"C:\\"` |
| `WARMUP_ONCE` | `pdf_viewer/mod.rs` | `Once::new()` |

### Configurações de Viewport

Definidas em `main.rs`:
```rust
egui::ViewportBuilder::default()
    .with_visible(false)           // Inicia oculto
    .with_maximized(false)         // Não maximizado inicialmente
    .with_inner_size([800.0, 600.0])
    .with_title("MTT File Manager")
    .with_app_id("mtt-file-manager")
    .with_decorations(false)       // Borderless
    .with_resizable(true)
```

---

## 7. Cache e Persistência

### Cache de Thumbnails em Disco

**Localização:** `{APPDATA}/mtt-file-manager/thumbnail_cache.db`

**Esquema SQLite:**
```sql
-- Inferido do código em disk_cache.rs
CREATE TABLE thumbnails (
    path TEXT,
    size INTEGER,
    data BLOB,
    mtime INTEGER,
    created_at INTEGER,
    PRIMARY KEY (path, size)
);
```

### Cache em Memória

Configurado via `LruCache`:
```rust
// Valores inferidos do código
LruCache::new(NonZeroUsize::new(1000).unwrap())  // ~1000 texturas
```

---

## 8. Fontes e Assets

### Fontes Utilizadas

1. **Segoe UI** - Fonte principal (Windows system font)
2. **Segoe UI Symbol** - Fallback para símbolos
3. **Arial Unicode MS** - Fallback Unicode (se disponível)
4. **Remix Icon** - Fonte de ícones (embarcada)

### Assets Embarcados

Via `include_bytes!()` em `embedded_assets.rs`:
- `remixicon.ttf` (603KB)
- `appicon.png` (343KB)
- 28 ícones SVG (total ~18KB)

---

## 9. Dependência de Hardware

### GPU

**Opcional mas recomendado:**
- Aceleração de vídeo via D3D11VA
- NVIDIA RTX VSR (opcional, via mpv)

### Codecs

O sistema depende de codecs instalados no Windows:
- Decodificação de vídeo via Media Foundation
- Consulta ao Registry para nomes de codecs

---

## 10. Configurações Não Documentadas

⚠️ **Itens que não possuem configuração explícita no código:**

1. **Tamanho de thumbnail** - Hardcoded (256px)
2. **Número de workers** - Baseado em CPU cores
3. **Timeout de operações** - Não configurável
4. **Limite de cache** - Hardcoded em memória e disco
5. **Idioma** - Hardcoded em Português (BR)
