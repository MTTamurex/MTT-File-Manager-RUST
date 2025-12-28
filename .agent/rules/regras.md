---
trigger: always_on
---

# ⚖️ LEI DO PROJETO - MTT File Manager

> **"Documentação viva é código vivo. Código sem documentação é código morto."**

Este arquivo contém as **regras inegociáveis** para desenvolvimento do MTT File Manager. Todos os agentes AI, desenvolvedores e contribuidores DEVEM seguir estas diretrizes rigorosamente.

---

## 🚨 REGRA DE OURO: Documentação Sincronizada

### PROTOCOLO OBRIGATÓRIO

**ANTES de fazer qualquer alteração no código, você DEVE:**

1. Ler a documentação relevante em `docs/`
2. Entender o contexto arquitetural
3. Planejar as mudanças

**DURANTE a alteração:**

4. Implementar o código
5. **IMEDIATAMENTE** atualizar a documentação correspondente
6. Verificar se há cross-references em outros docs

**APÓS a alteração:**

7. Revisar todos os arquivos em `docs/` afetados
8. Atualizar diagramas Mermaid se necessário
9. Commit com mensagem: `feat: X | docs: atualiza Y`

### Mapeamento: Código → Documentação

| Se você alterou... | Atualize... |
|-------------------|-------------|
| **Estrutura de pastas** | `docs/ARQUITETURA.md` (Seção "Estrutura de Pastas") |
| **Adicionar/remover dependência** | `docs/STACK.md` |
| **Windows APIs (unsafe blocks)** | `docs/SEGURANCA_WINDOWS.md` (Seção "Auditoria de Código Unsafe") |
| **Lógica de segurança** | `docs/SEGURANCA_WINDOWS.md` |
| **Performance optimization** | `docs/ARQUITETURA.md` (Seção "Performance Benchmarks") |
| **Novos débitos técnicos** | `docs/ROADMAP_TECNICO.md` |
| **Completar item do roadmap** | `docs/ROADMAP_TECNICO.md` (marcar ✅) |
| **Fluxo de dados** | `docs/ARQUITETURA.md` (Diagrama Mermaid) |

### ❌ PROIBIÇÕES ABSOLUTAS

**NUNCA:**
- Altere código sem atualizar documentação correspondente
- Deixe TODOs/FIXMEs no código sem registrar em `ROADMAP_TECNICO.md`
- Remova dependências sem atualizar `STACK.md`
- Adicione blocos `unsafe` sem documentar em `SEGURANCA_WINDOWS.md`
- Crie novas pastas/módulos sem atualizar diagrama de arquitetura

**VIOLAÇÃO DESTA REGRA = PR REJEITADO**

---

## 🧠 REGRA ANTI-ALUCINAÇÃO

### Checklist OBRIGATÓRIA Antes de Cada Alteração

```
[ ] Verifiquei se o arquivo existe (não inventei o path)
[ ] Confirmei que a biblioteca está em Cargo.toml
[ ] Testei localmente antes de sugerir
[ ] Não usei placeholders como "... existing code ..."
[ ] Todos os imports são de crates reais
[ ] Todos os tipos/structs foram declarados
```

### Bibliotecas PERMITIDAS

**Lista Branca de Dependências Atuais:**
```toml
eframe = "0.31"
rayon = "1.10"
walkdir = "2.5"
rfd = "0.15"
lru = "0.12"
windows = "0.58"
```

**Para adicionar NOVA dependência:**

1. Justifique no PR/commit message
2. Atualize `docs/STACK.md` ANTES do merge
3. Verifique compatibilidade de licença (MIT/Apache-2.0 apenas)
4. Analise impacto no tamanho do executável

### Proibições de Import

❌ **NUNCA** use:
```rust
use tokio::*;  // Não está em Cargo.toml
use async_std::*;  // Não está em Cargo.toml
use image::*;  // Não está em Cargo.toml (ainda)
```

✅ **SEMPRE** verifique primeiro:
```rust
// Comando para verificar: cargo tree | grep nome_crate
```

---

## ⚡ PERFORMANCE E MEMÓRIA (Desktop Long-Running Apps)

### Princípios para Apps Desktop

**Desktop ≠ Server ≠ CLI Tool**

Apps desktop rodam por HORAS/DIAS sem restart. Requisitos diferentes:

1. **Memória estável**: Sem crescimento linear ao longo do tempo
2. **Responsividade**: UI NUNCA pode travar (60 FPS mínimo)
3. **Recursos limitados**: Usuários rodam múltiplos apps

### Regras OBRIGATÓRIAS

#### 1. Lazy Loading em Tudo

❌ **ERRADO**:
```rust
// Carrega TUDO na memória
let all_thumbnails: Vec<Texture> = paths
    .iter()
    .map(|p| load_thumbnail(p))
    .collect();
```

✅ **CORRETO**:
```rust
// Carrega sob demanda no viewport
if is_visible_in_viewport(item) && !cache.contains(item) {
    request_load(item);
}
```

#### 2. Virtualização de Listas

❌ **ERRADO**:
```rust
for item in all_10000_items {
    render_thumbnail(item);  // Renderiza tudo!
}
```

✅ **CORRETO**:
```rust
ScrollArea::vertical()
    .show_rows(ui, row_height, total_rows, |ui, visible_range| {
        // Renderiza SOMENTE linhas visíveis
        for row in visible_range {
            render_row(ui, row);
        }
    });
```

#### 3. Cache com Limite (LRU)

❌ **ERRADO**:
```rust
// Cache infinito = OOM em 10 min
HashMap<PathBuf, Texture>
```

✅ **CORRETO**:
```rust
// Cache com eviction automática
LruCache<PathBuf, Texture>::new(500)
```

#### 4. Gerenciamento de Threads

❌ **ERRADO**:
```rust
// Cria thread por arquivo = 10k threads!
for file in files {
    std::thread::spawn(|| process(file));
}
```

✅ **CORRETO**:
```rust
// Thread pool com limite
const MAX_CONCURRENT: usize = 50;
if loading_set.len() < MAX_CONCURRENT {
    spawn_worker();
}

// Ou use rayon (thread pool automático)
files.par_iter().take(50).for_each(|f| process(f));
```

#### 5. Cleanup de Recursos

❌ **ERRADO**:
```rust
unsafe {
    let hbitmap = GetImage(...);
    // Esqueceu de deletar = memory leak!
}
```

✅ **CORRETO**:
```rust
unsafe {
    let hbitmap = GetImage(...);
    // ... uso ...
    DeleteObject(hbitmap);  // SEMPRE cleanup!
}

// Melhor ainda: Use RAII wrapper
struct HBitmapGuard(HBITMAP);
impl Drop for HBitmapGuard {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.0); }
    }
}
```

---

## 🔒 SEGURANÇA E ROBUSTEZ

### Regra: Zero Panics em Produção

❌ **PROIBIDO**:
```rust
let parent = path.parent().unwrap();
let texture = load_texture(...).expect("Failed");
let item = items[idx];  // Pode panic se idx out of bounds
```

✅ **OBRIGATÓRIO**:
```rust
let parent = path.parent()
    .ok_or(Error::NoParent)?;

let texture = load_texture(...)
    .unwrap_or_else(|_| placeholder_texture());

if let Some(item) = items.get(idx) {
    // Usa item
}
```

### Regra: Sanitize All User Input

**"User Input" Inclui:**
- Paths do sistema de arquivos
- Argumentos de linha de comando
- Arquivos de configuração
- Drag & drop
- Clipboard

❌ **ERRADO**:
```rust
fn navigate_to(&mut self, path: &str) {
    self.current_path = path.to_string();  // Path traversal!
    self.load_folder();
}
```

✅ **CORRETO**:
```rust
fn navigate_to(&mut self, path: &str) -> Result<()> {
    let sanitized = sanitize_path(path)?;
    self.current_path = sanitized.to_string_lossy().to_string();
    self.load_folder()?;
    Ok(())
}

fn sanitize_path(input: &str) -> Result<PathBuf> {
    use std::fs::canonicalize;
    
    let canonical = canonicalize(input)
        .map_err(|_| Error::InvalidPath)?;
    
    // Bloqueia paths sensíveis
    let forbidden = [
        r"C:\Windows\System32",
        r"C:\Windows\SysWOW64",
    ];
    
    for blocked in forbidden {
        if canonical.starts_with(blocked) {
            return Err(Error::ForbiddenPath);
        }
    }
    
    Ok(canonical)
}
```

### Regra: Error Handling Explícito

❌ **ERRADO**:
```rust
let _ = sender.send(data);  // Ignora erro!
```

✅ **CORRETO**:
```rust
if let Err(e) = sender.send(data) {
    error!("Failed to send thumbnail: {:?}", e);
    // Fallback ou retry logic
}
```

### Regra: Auditoria de Unsafe Blocks

**SEMPRE que adicionar `unsafe`:**

1. Documente a razão no código
2. Adicione entrada em `docs/SEGURANCA_WINDOWS.md`
3. Explique invariantes que você está garantindo

**Template**:
```rust
unsafe {
    // SAFETY: buffer tem tamanho validado acima (linha 123)
    // e pointer é válido porque vem de Vec::as_ptr()
    std::ptr::copy_nonoverlapping(src, dst, len);
}
```

---

## 📐 ESTILO DE CÓDIGO

### Formatação

**Use rustfmt (OBRIGATÓRIO):**
```powershell
cargo fmt --all
```

**Use clippy (OBRIGATÓRIO):**
```powershell
cargo clippy -- -D warnings
```

### Naming Conventions

```rust
// ✅ Correto
struct ImageViewerApp { }
const MAX_CACHE_SIZE: usize = 500;
fn load_folder(&self) { }
let thumbnail_data = extract_thumbnail();

// ❌ Errado
struct imageviewerapp { }
const maxCacheSize: usize = 500;
fn LoadFolder(&self) { }
let ThumbnailData = extract_thumbnail();
```

### Comentários

**SEMPRE comente:**
- Razão de decisões não-óbvias
- Workarounds para bugs externos
- Complexidade algorítmica
- Blocos `unsafe`

**NUNCA comente:**
- O óbvio (`i += 1; // Incrementa i`)
- Código comentado (delete!)

```rust
// ✅ BOM
// Windows thumbnail API retorna BGRA, mas egui espera RGBA.
// Fazemos swap manual dos canais vermelho e azul.
for pixel in buffer.chunks_exact_mut(4) {
    pixel.swap(0, 2);  // B ↔ R
}

// ❌ RUIM
// Loop pelos pixels
for pixel in buffer.chunks_exact_mut(4) {
    pixel.swap(0, 2);  // Troca posição 0 com 2
}
```

---

## 🧪 TESTES

### Regra: Test Coverage Mínima

**Toda nova funcionalidade DEVE ter testes.**

**Mínimos:**
- Funções públicas: 80% coverage
- Lógica de negócio: 90% coverage
- Código unsafe: 100% (quando possível mockar FFI)

### Categorias de Testes

```rust
// 1. Testes unitários (mesma file)
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sanitize_path() {
        assert!(sanitize_path("C:\\Users").is_ok());
        assert!(sanitize_path("C:\\..\\Windows").is_err());
    }
}

// 2. Testes de integração (tests/ folder)
// tests/integration_test.rs
#[test]
fn test_full_workflow() {
    let app = ImageViewerApp::default();
    app.navigate_to("C:\\Test");
    assert_eq!(app.items.len(), 10);
}

// 3. Testes de propriedades (proptest)
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_sanitize_never_panics(s in "\\PC*") {
        let _ = sanitize_path(&s);  // Não deve panic
    }
}
```

### Mocking de Windows APIs

```rust
#[cfg(test)]
use mockall::mock;

#[cfg(not(test))]
fn extract_windows_thumbnail(path: &Path) -> Result<Thumbnail> {
    // Código real com Windows APIs
}

#[cfg(test)]
fn extract_windows_thumbnail(path: &Path) -> Result<Thumbnail> {
    // Mock retorna fixture
    Ok(Thumbnail::test_fixture())
}
```

---

## 📝 COMMITS E PRs

### Formato de Commit Messages

```
<tipo>: <descrição curta> | docs: <atualização de docs>

<corpo opcional>

Closes #123
```

**Tipos:**
- `feat`: Nova funcionalidade
- `fix`: Correção de bug
- `perf`: Melhoria de performance
- `refactor`: Refatoração sem mudança de comportamento
- `docs`: Só documentação
- `test`: Adiciona/corrige testes
- `chore`: Build, CI, dependências

**Exemplos:**
```
feat: adiciona busca de arquivos | docs: atualiza ARQUITETURA.md com novo fluxo

Implementa barra de busca com filtro em tempo real.
Atualiza diagrama Mermaid com novo componente SearchBar.

Closes #45
```

```
fix: previne path traversal em navigate_to | docs: atualiza SEGURANCA_WINDOWS.md

Adiciona sanitize_path() que usa canonicalize() e bloqueia
paths sensíveis do sistema.

Documenta nova função em seção de segurança.
```

### Pull Request Checklist

**Antes de abrir PR, confirme:**

- [ ] Código compila sem warnings (`cargo build --release`)
- [ ] Passou em `cargo clippy`
- [ ] Passou em `cargo fmt --check`
- [ ] Testes passam (`cargo test`)
- [ ] Documentação atualizada em `docs/`
- [ ] README.md atualizado se necessário
- [ ] Commit messages seguem formato
- [ ] Sem TODOs/FIXMEs não documentados

---

## 🚀 BUILD E RELEASE

### Debug Build (Desenvolvimento)

```powershell
cargo build
cargo run
```

### Release Build (Produção)

```powershell
cargo build --release
# Executável em: target\release\mtt-file-manager.exe

# Strip symbols (reduz mais 20-30%)
strip target\release\mtt-file-manager.exe
```

### Tamanho do Executável

**Limites:**
- Debug: <50 MB aceitável
- Release: <10 MB (ideal: 4-6 MB)

**Se exceder, investigue:**
```powershell
cargo bloat --release --crates
```

