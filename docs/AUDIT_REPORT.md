# 📋 RELATÓRIO DE AUDITORIA COMPLETA

## MTT File Manager - Análise Arquitetural (egui/eframe)

**Data da Auditoria**: Janeiro 2026  
**Auditor**: Arquiteto de Software Sênior - Especialista em Rust e GUI Immediate Mode

---

## 1. Visão Geral e Stack

### 1.1 Propósito da Aplicação

Um **gerenciador de arquivos nativo para Windows** que foca em visualização de thumbnails de alta performance para imagens e vídeos, usando APIs nativas do Windows (Shell, WIC, Media Foundation).

### 1.2 Análise de Dependências (`Cargo.toml`)

| Dependência | Versão | Propósito |
|-------------|--------|-----------|
| `eframe` | 0.31 | Framework egui com persistence |
| `rayon` | 1.10 | Paralelismo para ordenação de listas grandes |
| `walkdir` | 2.5 | Iteração recursiva em diretórios |
| `notify` | 6.1.1 | File system watcher (auto-refresh) |
| `lru` | 0.12 | Cache LRU para texturas e ícones |
| `dashmap` | 5.5 | Concurrent HashMap (não usado ativamente) |
| `image` | 0.25 | Decodificação de imagens |
| `rusqlite` | 0.32 | Cache SQLite persistente |
| `webp` | 0.3 | Compressão lossy de thumbnails |
| `windows` | 0.58 | APIs Win32 (Shell, COM, Media Foundation) |
| `resvg/usvg` | 0.44 | Renderização de ícones SVG |

**Observação**: A stack é bem escolhida para o propósito, com destaque para `windows-rs` que permite acesso direto às APIs nativas sem overhead de FFI manual.

### 1.3 Configuração de Build

```toml
[profile.release]
opt-level = 3      # ✅ Máxima otimização
lto = true         # ✅ Link-Time Optimization
codegen-units = 1  # ✅ Melhor inlining cross-crate
```

**✅ Bem configurado** - Produz executável ~4-6MB com performance otimizada.

---

## 2. Arquitetura Específica para Immediate Mode

### 2.1 Separação Lógica vs. UI

| Critério | Status | Observação |
|----------|--------|------------|
| Lógica de negócios isolada | ⚠️ **Parcial** | `main.rs` tem **~5000 linhas** com UI e lógica misturadas |
| Domain layer | ✅ **Bom** | `domain/` contém `FileEntry`, `SortMode`, `ThumbnailData` |
| Infrastructure layer | ✅ **Bom** | `infrastructure/` isola Windows APIs |
| Workers assíncronos | ✅ **Excelente** | Thumbnails, metadata, folder scan em threads separadas |

**⚠️ Anti-pattern Identificado**: O arquivo `main.rs` é um "God Object" com `ImageViewerApp` contendo:
- 50+ campos de estado
- Funções de renderização UI
- Lógica de navegação
- Clipboard handling
- Context menu
- Todos em um único arquivo

### 2.2 Gerenciamento de Estado

```rust
struct ImageViewerApp {
    // Estado de UI (~15 campos)
    current_path: String,
    selected_item: Option<usize>,
    view_mode: ViewMode,
    thumbnail_size: f32,
    // ...
    
    // Canais de Workers (~10 canais)
    thumbnail_req_sender: Sender<(PathBuf, usize)>,
    image_receiver: Receiver<ThumbnailData>,
    // ...
    
    // Caches (~8 caches diferentes)
    cache_manager: CacheManager,
    metadata_cache: LruCache<...>,
    // ...
}
```

**Persistência**: Usa SQLite via `disk_cache.rs` para:
- Thumbnails comprimidos (WebP)
- Preferências do usuário
- Capas de pastas

✅ **Bem implementado** - Não usa `eframe::Storage` (muito limitado), preferindo SQLite próprio.

### 2.3 Modularização

| Módulo | Linhas | Responsabilidade |
|--------|--------|------------------|
| `main.rs` | ~5000 | **Monolítico** - precisa quebrar |
| `ui/views/grid_view.rs` | ~472 | ✅ Bem isolado |
| `ui/sidebar.rs` | ~355 | ✅ Bem isolado |
| `workers/thumbnail_worker.rs` | ~750 | ✅ Worker completo |
| `infrastructure/disk_cache.rs` | ~427 | ✅ Persistência isolada |

**⚠️ Problema**: Existe uma tentativa de refatoração em `ui/app.rs` e `application/state.rs`, mas estes arquivos **não são usados** - o código real ainda está em `main.rs`.

---

## 3. Performance Crítica (Update Loop)

### 3.1 Bloqueio da Thread Principal

| Operação | Status | Localização |
|----------|--------|-------------|
| Scan de pasta | ✅ **Assíncrono** | `load_folder()` usa `std::thread::spawn` |
| Carregamento de thumbnails | ✅ **Assíncrono** | Worker pool com 4 threads |
| Extração de metadados | ✅ **Assíncrono** | Worker dedicado |
| Refresh de drives | ⚠️ **Síncrono** | `get_all_drives()` no main thread (rápido, OK) |
| Ordenação | ✅ **Paralelo p/ >5000 itens** | Usa `rayon::par_sort_by` |

**✅ Excelente**: Nenhuma operação de I/O pesada no loop `update()`.

### 3.2 Alocações no Hot Path

```rust
// PROBLEMA: Clone de Vec a cada frame para evitar borrow checker
let items = self.items.clone(); // Arc clone (barato, OK)
let selected_file = self.selected_file.clone(); // FileEntry clone (evitável)
```

**Mitigação existente**: 
- `items: Arc<Vec<FileEntry>>` - Clone do Arc é O(1)
- Tooltips usam `.clone()` do item apenas quando hover

**⚠️ Potencial melhoria**: Usar índices ao invés de clonar `selected_file`.

### 3.3 Repaint Request

```rust
// ✅ CORRETO: Apenas quando há dados novos
if received_any {
    ctx.request_repaint();
}

// ✅ CORRETO: Workers disparam repaint após enviar dados
ctx.request_repaint(); // Em thumbnail_worker_loop após enviar resultado
```

**✅ Bem implementado** - Não há `request_repaint()` incondicional.

---

## 4. Qualidade de Código e "Rust Idioms"

### 4.1 Uso Idiomático do Rust

| Critério | Status |
|----------|--------|
| Pattern Matching | ✅ Usado extensivamente |
| Option/Result handling | ⚠️ Alguns `.unwrap()` em paths não críticos |
| Iterators | ✅ Preferidos sobre loops |
| Lifetimes | ✅ Evitados via clones quando necessário |

### 4.2 Thread Safety

```rust
// ✅ Correto: Arc<AtomicUsize> para tracking de geração
current_generation: Arc<AtomicUsize>,

// ✅ Correto: Mutex apenas onde necessário
shared_req_rx: Arc<Mutex<Receiver<...>>>,

// ✅ Correto: mpsc channels para comunicação unidirecional
thumbnail_req_sender: Sender<(PathBuf, usize)>,
```

**✅ Bem implementado** - Uso correto de primitivas de sincronização.

### 4.3 Tratamento de Erros

```rust
// ⚠️ PROBLEMA: Erros silenciosos em alguns lugares
let _ = FindClose(handle); // Ignora erro
let _ = conn.execute("PRAGMA...", []); // Ignora erro de DB

// ✅ BOM: Sistema de notificações toast
self.notifications.push(
    AppNotification::error(format!("Erro ao restaurar: {}", e)),
);
```

**⚠️ Melhoria**: Implementar logging estruturado (tracing) ao invés de `eprintln!`.

---

## 5. UI/UX e Layout

### 5.1 Estrutura de Layout

```rust
// Hierarquia bem definida
egui::TopBottomPanel::top("tab_bar")...  // Tab bar (custom title bar)
egui::TopBottomPanel::top("nav_bar")...  // Navigation toolbar
egui::SidePanel::left("sidebar")...       // Drives + Quick access
egui::SidePanel::right("preview_panel")... // Details pane
egui::CentralPanel::default()...           // Grid/List content
```

**✅ Layout lógico** similar ao Windows Explorer.

### 5.2 Constantes e Estilo

```rust
// ⚠️ PROBLEMA: Magic numbers espalhados
let padding = 8.0;
let icon_size = 22.0;
let button_size = egui::vec2(size + padding * 2.0, size + padding * 2.0);

// ⚠️ PROBLEMA: Cores hardcoded
Color32::from_rgb(200, 220, 240) // Selection color
Color32::from_rgb(45, 45, 45)    // Dark mode background
```

**⚠️ Melhoria**: Criar módulo `ui/theme.rs` com constantes centralizadas.

### 5.3 IDs do egui

```rust
// ✅ CORRETO: IDs únicos para elementos
egui::Area::new(egui::Id::new("resize_grip"))...
egui::ComboBox::from_id_salt("sort_mode")...
ui.interact(item_rect, ui.id().with(index), Sense::click())
```

**✅ Bem implementado** - IDs manuais onde necessário.

---

## 6. Pontos Fortes e Fracos

### ✅ Pontos Fortes (Highs)

1. **Arquitetura de Workers**
   - Pool de 4 threads para thumbnails com controle de concorrência (`MAX_CONCURRENT_DECODES = 4`)
   - Sistema de "geração" para cancelar operações obsoletas
   - Cache em dois níveis (memória LRU + SQLite persistente)

2. **Integração Windows Nativa**
   - Uso direto de `IShellItemImageFactory`, `IContextMenu`, Media Foundation
   - Detecção de OneDrive com sync status
   - Menu de contexto com extensões do shell

3. **Performance de UI**
   - Zero I/O no render loop
   - Virtualização de grid (apenas itens visíveis renderizados)
   - Lazy loading de thumbnails

4. **Persistência de Estado**
   - SQLite para preferências, cache de thumbnails, capas de pastas
   - Restaura tamanho de janela, largura de sidebars, modo de visualização

### ⚠️ Pontos Fracos (Lows)

1. **Arquivo `main.rs` Monolítico (~5000 linhas)**
   - Violação do princípio de responsabilidade única
   - Dificulta testes unitários
   - Causa conflitos em merges

2. **Refatoração Incompleta**
   - `ui/app.rs` e `application/state.rs` existem mas **não são usados**
   - Código duplicado entre versões

3. **Tratamento de Erros Inconsistente**
   - Mix de `.unwrap()`, `let _ =`, e `eprintln!`
   - Sem logging estruturado

4. **Magic Numbers**
   - Cores, tamanhos, paddings hardcoded
   - Dificulta theming

5. **Código em Português/Inglês Misturado**
   - Comentários e variáveis em português (`filtrar_items`, `Lixeira`)
   - UI strings hardcoded (dificulta i18n)

---

## 7. Sugestões de Melhoria (Roadmap)

### Prioridade Alta (Dívida Técnica)

1. **Quebrar `main.rs` em módulos**
   - Extrair `impl eframe::App` para `ui/app_impl.rs`
   - Mover renderização de views para `ui/views/`
   - Criar `app/commands.rs` para ações (copy, paste, delete)

2. **Completar refatoração de estado**
   - Usar `AppState` de `application/state.rs`
   - Implementar traits para operações (`ClipboardOps`, `NavigationOps`)

3. **Centralizar constantes de UI**
   ```rust
   // ui/theme.rs
   pub const PADDING_SM: f32 = 4.0;
   pub const PADDING_MD: f32 = 8.0;
   pub const COLOR_SELECTION: Color32 = Color32::from_rgb(200, 220, 240);
   ```

### Prioridade Média (Qualidade)

4. **Implementar logging com `tracing`**
   ```rust
   tracing::info!(path = %folder_path, "Starting folder scan");
   tracing::error!(error = %e, "Thumbnail extraction failed");
   ```

5. **Testes unitários**
   - Testar `sort_items()`, `filter_items()` isoladamente
   - Mock de `CacheManager` para testes de UI

6. **Extrair strings para i18n**
   ```rust
   // i18n/pt_BR.rs
   pub const RECYCLE_BIN: &str = "Lixeira";
   pub const THIS_PC: &str = "Este Computador";
   ```

### Prioridade Baixa (Nice-to-Have)

7. **Adicionar CI/CD**
   - GitHub Actions para `cargo clippy`, `cargo fmt`, `cargo test`

8. **Documentação inline**
   - `///` docs para funções públicas
   - Exemplos de uso em módulos

---

## 8. Métricas do Projeto

### Estatísticas de Código

| Arquivo/Módulo | Linhas | Complexidade |
|----------------|--------|--------------|
| `src/main.rs` | ~5000 | ⚠️ Alta |
| `src/workers/thumbnail_worker.rs` | ~750 | Média |
| `src/ui/views/grid_view.rs` | ~472 | Média |
| `src/infrastructure/disk_cache.rs` | ~427 | Baixa |
| `src/ui/sidebar.rs` | ~355 | Baixa |
| `src/ui/cache.rs` | ~369 | Baixa |
| `src/application/state.rs` | ~304 | Baixa |

### Dependências Windows APIs

| API | Uso |
|-----|-----|
| `IShellItemImageFactory` | Thumbnails nativos |
| `IContextMenu` | Menu de contexto do shell |
| `Media Foundation` | Metadados de vídeo |
| `WIC (Windows Imaging Component)` | Decodificação HEIC/AVIF |
| `SHFileOperationW` | Copiar/Mover/Excluir com Undo |
| `FindFirstFileW/FindNextFileW` | Scan de diretórios |

---

## 9. Conclusão

O **MTT File Manager** é um projeto bem arquitetado para performance, com excelente uso de workers assíncronos e integração Windows nativa. A principal dívida técnica é o arquivo `main.rs` monolítico que precisa ser quebrado em módulos menores para facilitar manutenção e testes.

### Nota Geral: ⭐⭐⭐⭐ (4/5)

| Critério | Nota |
|----------|------|
| **Performance** | ⭐⭐⭐⭐⭐ (5/5) |
| **Arquitetura** | ⭐⭐⭐⭐ (4/5) |
| **Manutenibilidade** | ⭐⭐⭐ (3/5) |
| **Qualidade de Código** | ⭐⭐⭐⭐ (4/5) |
| **Integração Windows** | ⭐⭐⭐⭐⭐ (5/5) |

### Recomendação Final

O projeto está em um bom estado para uso, mas precisa de uma **refatoração planejada** do `main.rs` antes de adicionar novas features significativas. Sugerimos:

1. Congelar novas features por 1-2 sprints
2. Quebrar `main.rs` em módulos (prioridade 1-3 do roadmap)
3. Adicionar testes unitários para código extraído
4. Retomar desenvolvimento de features

---

*Relatório gerado em Janeiro 2026*
