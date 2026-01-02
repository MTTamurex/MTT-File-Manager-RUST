# 📊 Relatório de Auditoria Técnica - MTT File Manager

**Data:** 02/01/2026  
**Versão:** 0.1.0  
**Linguagem:** Rust  
**Framework UI:** eframe/egui  
**Plataforma:** Windows (Win32 API)

---

## 📋 Resumo Executivo

| Categoria | Nota | Comentário |
|-----------|------|------------|
| **Qualidade de Código** | 6/10 | God Object em `main.rs` (2525 linhas), refatoração incompleta |
| **Arquitetura** | 5/10 | Separação de camadas iniciada mas inconsistente |
| **Performance** | 7.5/10 | Boas práticas de async, LRU cache, thread pools |
| **Segurança** | 6.5/10 | Módulo de segurança existe mas não é usado |
| **Bugs Potenciais** | 6/10 | Vários pontos de panic, edge cases não tratados |
| **NOTA GLOBAL** | **6.2/10** | Projeto funcional com débitos técnicos significativos |

### 🎯 Pontos Positivos
- ✅ Sistema de cache em camadas (LRU + SQLite) bem implementado
- ✅ Worker pool com controle de concorrência
- ✅ Separação inicial em módulos (domain, infrastructure, application, ui)
- ✅ Testes unitários em módulos críticos (security, cache, errors)
- ✅ Regras de projeto bem definidas em `.cursorrules`
- ✅ Tratamento de erros com `thiserror` e tipos personalizados

### ⚠️ Pontos Críticos
- ❌ `main.rs` com 2525 linhas (God Object anti-pattern)
- ❌ Módulo de segurança (`security.rs`) não é utilizado no código principal
- ❌ Blocos `unsafe` sem documentação completa de invariantes
- ❌ `.unwrap()` e `.expect()` em código de produção
- ❌ Arquivos backup (`.rs.bak`, `.rs.backup`) no repositório

---

## 📊 Tabela de Prioridades

| ID | Issue | Impacto | Esforço | Prioridade |
|----|-------|---------|---------|------------|
| C01 | God Object em `main.rs` | 🔴 Alto | 🔴 Alto | **P1** |
| C02 | Módulo security não utilizado | 🔴 Alto | 🟡 Médio | **P1** |
| C03 | `.unwrap()`/`.expect()` em produção | 🔴 Alto | 🟡 Médio | **P1** |
| C04 | Blocos `unsafe` sem SAFETY docs | 🟡 Médio | 🟢 Baixo | **P2** |
| C05 | Race condition em file watcher | 🟡 Médio | 🟡 Médio | **P2** |
| M01 | Duplicação de código ListView/GridView | 🟡 Médio | 🟡 Médio | **P2** |
| M02 | Config hardcoded (cache sizes) | 🟢 Baixo | 🟢 Baixo | **P3** |
| M03 | Logging inconsistente (eprintln!) | 🟢 Baixo | 🟢 Baixo | **P3** |
| M04 | Arquivos backup no repositório | 🟢 Baixo | 🟢 Baixo | **P3** |

---

## 🔴 Análise Crítica (Bugs/Segurança)

### C01: God Object - `main.rs` com 2525 linhas

**Arquivo:** [main.rs](file:///c:/MTT%20File%20Manager/src/main.rs)

A struct `ImageViewerApp` contém **35+ campos** e **40+ métodos**, violando o Princípio da Responsabilidade Única (SRP).

```rust
// Exemplo do problema (linhas 94-186)
struct ImageViewerApp {
    // Estado de navegação (deveria estar em NavigationState)
    current_path: String,
    navigation_history: Vec<String>,
    history_index: usize,
    
    // Estado de cache (deveria estar em CacheState)
    thumbnail_req_sender: Sender<...>,
    image_receiver: Receiver<...>,
    cache_manager: CacheManager,
    
    // Estado de UI (deveria estar em UIState)
    thumbnail_size: f32,
    selected_item: Option<usize>,
    show_preview_panel: bool,
    
    // ... + 25 outros campos misturados
}
```

**Solução Proposta:**

```rust
// ANTES: Uma struct monolítica
struct ImageViewerApp { /* 35 campos */ }

// DEPOIS: Composição de estados
struct ImageViewerApp {
    navigation: NavigationState,
    cache: CacheState,
    ui: UIState,
    workers: WorkerChannels,
    clipboard: ClipboardState,
}

struct NavigationState {
    current_path: String,
    history: Vec<String>,
    history_index: usize,
    is_computer_view: bool,
}

struct CacheState {
    texture_cache: LruCache<PathBuf, TextureHandle>,
    icon_cache: LruCache<String, TextureHandle>,
    loading_set: HashSet<PathBuf>,
}

struct WorkerChannels {
    thumbnail_sender: Sender<(PathBuf, usize)>,
    thumbnail_receiver: Receiver<ThumbnailData>,
    cover_sender: Sender<PathBuf>,
    // ...
}
```

---

### C02: Módulo de Segurança Não Utilizado

**Arquivo:** [security.rs](file:///c:/MTT%20File%20Manager/src/infrastructure/security.rs)

O módulo `security.rs` implementa sanitização de paths, mas **nunca é chamado** no código principal:

```rust
// security.rs (implementado mas não usado)
pub fn sanitize_path(path: &Path, config: &SecurityConfig) -> Result<PathBuf, SecurityError>

// main.rs (navega para paths SEM sanitização!)
fn navigate_to(&mut self, path: &str) {
    // ❌ Nenhuma validação de segurança!
    let normalized_path = if path.len() >= 2 && path.chars().nth(1) == Some(':') {
        // Apenas normalização básica, sem sanitização
        // ...
    };
    self.current_path = normalized_path;
    self.load_folder(false);
}
```

**Solução Proposta:**

```rust
// ANTES (main.rs linha 821)
fn navigate_to(&mut self, path: &str) {
    let normalized_path = /* ... */;
    self.current_path = normalized_path;
}

// DEPOIS (com sanitização)
use crate::infrastructure::security::{sanitize_path, SecurityConfig};

fn navigate_to(&mut self, path: &str) -> Result<(), SecurityError> {
    let config = SecurityConfig::default();
    let safe_path = sanitize_path(Path::new(path), &config)?;
    
    self.current_path = safe_path.to_string_lossy().to_string();
    self.load_folder(false);
    Ok(())
}
```

---

### C03: Uso de `.unwrap()` e `.expect()` em Produção

**Arquivos Afetados:** `main.rs`, `disk_cache.rs`, `thumbnail_worker.rs`

```rust
// disk_cache.rs linha 32 - CRÍTICO
let conn = Connection::open(db_path).expect("Failed to open thumbnail database");
// ❌ Crash total se não conseguir abrir o banco!

// main.rs linha 2509-2511
fonts.families.get_mut(&egui::FontFamily::Proportional)
    .unwrap()  // ❌ Panic se não existir!
    .extend(loaded_fonts.clone());

// thumbnail_worker.rs linha 56-58
let work = match rx.lock() {
    Ok(lock) => lock.recv(),
    Err(_) => break,  // ✅ Este está correto
};
```

**Solução:**

```rust
// ANTES (disk_cache.rs)
let conn = Connection::open(db_path).expect("Failed to open thumbnail database");

// DEPOIS (graceful degradation)
let conn = match Connection::open(db_path) {
    Ok(c) => c,
    Err(e) => {
        eprintln!("[Cache] Failed to open database: {:?}", e);
        // Fallback: use in-memory database
        Connection::open_in_memory()
            .expect("[FATAL] Cannot create even in-memory database")
    }
};
```

---

### C04: Blocos `unsafe` sem Documentação de SAFETY

**Arquivo:** [main.rs](file:///c:/MTT%20File%20Manager/src/main.rs#L414-424)

```rust
// main.rs linhas 414-424
unsafe {
    let result = SHFileOperationW(&mut op);  // ❌ Falta SAFETY comment!
    if result == 0 {
        self.disk_cache.remove_cache_for_path(&path);
        self.selected_item = None;
        self.selected_file = None;
    }
}
```

**Correção:**

```rust
// SAFETY: 
// 1. `op` é inicializado corretamente com todos os campos obrigatórios
// 2. `from_vec` permanece válido durante toda a chamada (não é dropado)
// 3. A API SHFileOperationW é thread-safe para uso em single thread
// 4. Resultado 0 indica sucesso conforme documentação do Windows
unsafe {
    let result = SHFileOperationW(&mut op);
    // ...
}
```

---

### C05: Race Condition no File Watcher

**Arquivo:** [main.rs](file:///c:/MTT%20File%20Manager/src/main.rs#L1146-1176)

```rust
// O watcher envia eventos, mas o debounce de 500ms pode causar perda de eventos
while let Ok(event) = self.fs_event_receiver.try_recv() {
    match event {
        Ok(evt) => {
            // ❌ Se múltiplos eventos chegarem em 500ms, só o primeiro é processado
            self.pending_auto_reload = true;
        },
        Err(e) => eprintln!("Erro de watch: {:?}", e),
    }
}

// Debounce ignora eventos intermediários
if self.pending_auto_reload {
    let elapsed = self.last_auto_reload.elapsed();
    if elapsed > Duration::from_millis(500) {
        self.load_folder(true);  // ❌ Pode perder eventos Create seguidos de Modify
```

**Solução:** Usar um sistema de batch de eventos em vez de simples flag boolean.

---

## 🟡 Melhorias (Refatoração/Performance)

### M01: Duplicação de Código ListView/GridView

**Arquivos:** `grid_view.rs`, `list_view.rs`, `main.rs`

As ações `GridAction` e `ListAction` são **idênticas**:

```rust
// main.rs linhas 1319-1325 (ListAction)
enum ListAction {
    NavigateTo(String),
    OpenWithShell(PathBuf),
    RequestThumbnailLoad(PathBuf),
    RequestFolderScan(PathBuf),
    RenameWithShell(usize),
}

// main.rs linhas 1526-1532 (GridAction)
enum GridAction {
    NavigateTo(String),
    OpenWithShell(PathBuf),
    RequestThumbnailLoad(PathBuf),
    RequestFolderScan(PathBuf),
    RenameWithShell(usize),  // ❌ 100% idêntico!
}
```

**Solução:**

```rust
// src/ui/views/common.rs (já existe mas subutilizado)
pub enum ViewAction {
    NavigateTo(String),
    OpenWithShell(PathBuf),
    RequestThumbnailLoad(PathBuf),
    RequestFolderScan(PathBuf),
    RenameWithShell(usize),
}
```

---

### M02: Configurações Hardcoded

```rust
// main.rs linhas 72-75
const CACHE_SIZE: usize = 200;  // ❌ Hardcoded
const ICON_CACHE_SIZE: usize = 100;  // ❌ Hardcoded

// thumbnail_worker.rs linha 17
const MAX_CONCURRENT_DECODES: usize = 4;  // ❌ Hardcoded

// disk_cache.rs linha 139
if width > 512 || height > 512  // ❌ Magic number
```

**Solução:** Criar `config.rs` com todas as constantes externalizadas.

---

### M03: Logging Inconsistente

```rust
// Alguns lugares usam eprintln! (não profissional)
eprintln!("[GC] Removed {} orphaned cache entries", removed);  // disk_cache.rs
eprintln!("[Cache] Cleaned {} entries for: {}", deleted, path_str);  // disk_cache.rs

// errors.rs tem macros de tracing, mas não são usadas
#[macro_export]
macro_rules! safe_unwrap {
    ($expr:expr, $context:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("{}: {:?}", $context, e);  // ← Definido mas não usado
```

**Solução:** Adicionar `tracing` ao `Cargo.toml` e substituir todos os `eprintln!`.

---

## 🟢 Boas Práticas (Nitpicking)

### BP01: Arquivos de Backup no Repositório

```
src/main.rs.backup (150KB)
src/main.rs.refactored
src/main_functional_backup.rs (136KB)
src/ui/components.rs.bak
src/ui/icon_loader.rs.bak
src/ui/render_drive_slot.rs.bak
src/ui/render_item_slot.rs.bak
src/ui/texture_cache.rs.bak
```

**Solução:** Adicionar ao `.gitignore`:

```gitignore
*.bak
*.backup
*.refactored
*_backup.rs
```

---

### BP02: Comentários em Português Misturados com Inglês

```rust
// Caminho padrão (PT)
const PATH_PADRAO: &str = "C:\\";

// Persistent SQLite cache for thumbnails (EN)
pub struct ThumbnailDiskCache { ... }

/// Salva as preferências atuais no SQLite (PT)
fn save_preferences(&self) { ... }
```

**Solução:** Padronizar em um único idioma (preferencialmente inglês para código público).

---

### BP03: Imports Redundantes

```rust
// main.rs linhas 50-52 - import duplicado
use windows::Win32::Storage::FileSystem::{
    FindFirstFileW, FindNextFileW, FindClose, WIN32_FIND_DATAW, FILE_ATTRIBUTE_DIRECTORY
};
// Já importado indiretamente via windows::Win32::Storage::FileSystem::* (linha 45)
```

---

## 📈 Métricas do Código

| Arquivo | Linhas | Complexidade | Observação |
|---------|--------|--------------|------------|
| `main.rs` | 2525 | 🔴 Muito Alta | God Object |
| `list_view.rs` | 450 | 🟡 Média | OK |
| `item_slot.rs` | 500 | 🟡 Média | OK, bem documentado |
| `thumbnail_worker.rs` | 316 | 🟢 Baixa | Excelente |
| `disk_cache.rs` | 323 | 🟢 Baixa | Bem estruturado |
| `security.rs` | 275 | 🟢 Baixa | Com testes! |
| `icons.rs` | 294 | 🟢 Baixa | <300 linhas (meta atingida) |

---

## 🎯 Plano de Ação Recomendado

### Fase 1: Correções Críticas (1-2 dias)
1. [ ] Integrar `sanitize_path()` em `navigate_to()`
2. [ ] Substituir `.expect()` em `disk_cache.rs` por fallback
3. [ ] Adicionar SAFETY comments a todos os blocos `unsafe`

### Fase 2: Refatoração Estrutural (3-5 dias)
1. [ ] Extrair `NavigationState` de `ImageViewerApp`
2. [ ] Extrair `ClipboardState` de `ImageViewerApp`
3. [ ] Unificar `GridAction`/`ListAction` em `ViewAction`
4. [ ] Remover arquivos `.bak` e `.backup`

### Fase 3: Qualidade de Código (2-3 dias)
1. [ ] Adicionar `tracing` e substituir `eprintln!`
2. [ ] Criar `config.rs` para constantes
3. [ ] Padronizar idioma dos comentários (EN)
4. [ ] Rodar `cargo clippy -- -D warnings`

---

## 📎 Referências

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [The Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [Windows Crate Documentation](https://microsoft.github.io/windows-docs-rs/)

---

*Relatório gerado por análise automatizada de código. Revisão humana recomendada.*
