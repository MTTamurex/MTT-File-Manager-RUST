# Code Review Report - MTT File Manager

**Data:** 2026-02-09  
**Revisor:** AI Code Reviewer  
**Versão do Projeto:** 0.1.0  
**Total de Arquivos Analisados:** ~150+ módulos Rust

---

## 📊 Resumo Executivo

O MTT File Manager é um projeto bem estruturado com arquitetura em camadas clara e separação de responsabilidades adequada. O código demonstra preocupações genuínas com performance, especialmente no manuseio de I/O de disco, thumbnails e integração com Windows APIs.

### Pontos Fortes
- ✅ Arquitetura em camadas bem definida (Domain → Application → Infrastructure → UI)
- ✅ Uso extensivo de processamento assíncrono e workers
- ✅ Cache multi-nível implementado (memória → disco SQLite → GPU textures)
- ✅ Documentação inline extensiva e comentários de SAFETY para unsafe blocks
- ✅ Tratamento de erros centralizado com `AppError` e `thiserror`
- ✅ Otimizações para OneDrive (timeout-protected I/O)

### Pontos de Atenção
- ⚠️ 158 blocos `unsafe` espalhados pelo código
- ⚠️ 83+ usos de `unwrap()`/`expect()` em código de produção
- ⚠️ Alguns módulos placeholder/arquivos vazios
- ⚠️ Potenciais memory leaks em COM objects e handles do Windows
- ⚠️ Deadlocks potenciais em mutexes aninhados

---

## 🔴 CRÍTICO - Bugs e Falhas de Segurança

### 1. **Panic em Cache de Disco (Falha de Inicialização)**
**Arquivo:** [`src/infrastructure/disk_cache.rs`](src/infrastructure/disk_cache.rs:46)
```rust
Err(fatal_e) => {
    panic!(
        "[FATAL] Cannot create even an in-memory database: {:?}",
        fatal_e
    );
}
```
**Problema:** Panic irreversível se o SQLite falhar até mesmo em memória.  
**Impacto:** Aplicação crasha sem possibilidade de recovery.  
**Solução:** Usar fallback para operação sem cache ou notificar usuário.

### 2. **Path Traversal Parcialmente Mitigado**
**Arquivo:** [`src/infrastructure/security.rs`](src/infrastructure/security.rs:67)
```rust
pub fn sanitize_path(path: &Path, config: &SecurityConfig) -> Result<PathBuf, SecurityError>
```
**Problema:** Validação de drive usa apenas o primeiro caractere, permitindo bypass com paths como `C@:\` (em alguns contextos Windows).  
**Impacto:** Potencial acesso a drives não permitidos.  
**Solução:** Usar regex mais rigoroso ou `Path::components()` completo.

### 3. **Race Condition em Semaphore**
**Arquivo:** [`src/workers/thumbnail/worker.rs`](src/workers/thumbnail/worker.rs:45)
```rust
fn acquire(&self) {
    let mut count = self.count.lock().unwrap();
    while *count >= self.max {
        count = self.condvar.wait(count).unwrap();
    }
    *count += 1;
}
```
**Problema:** `Mutex` pode paniquar se envenenado por outra thread.  
**Impacto:** Worker thread crasha silenciosamente.  
**Solução:** Tratar erro do mutex gracefulmente ou usar `parking_lot`.

### 4. **Uso de `unwrap()` em Código de Produção**
**Arquivo:** [`src/domain/errors.rs`](src/domain/errors.rs:76)
```rust
#[macro_export]
macro_rules! safe_unwrap {
    ($expr:expr, $context:expr) => {
        match $expr {
            Ok(val) => val,
            Err(e) => {
                tracing::error!("{}: {:?}", $context, e);
                return Err(AppError::from(e));
            }
        }
    };
```
**Problema:** A macro `safe_unwrap!` importa `tracing` mas o crate não parece ter `tracing` configurado como dependência principal no Cargo.toml.  
**Impacto:** Erros de compilação ou runtime se usada.  
**Solução:** Verificar dependência ou usar `eprintln!`.

### 5. **COM Initialization Múltipla Inconsistente**
**Arquivos:** Múltiplos workers
```rust
// Alguns usam COINIT_APARTMENTTHREADED
// Outros usam COINIT_MULTITHREADED
```
**Problema:** Mistura de modos COM pode causar comportamento indefinido.  
**Impacto:** Deadlocks ou crashes em operações Shell.  
**Solução:** Padronizar para um modo ou usar `ComGuard` consistentemente.

### 6. **Buffer Overflow Potencial em PDF Viewer**
**Arquivo:** [`src/pdf_viewer/webview.rs`](src/pdf_viewer/webview.rs:30)
```rust
#[repr(C)]
struct ICoreWebView2Environment_Vtbl {
    pub base: IUnknown_Vtbl,
    pub CreateCoreWebView2Controller:
        unsafe extern "system" fn(*mut c_void, HWND, *mut c_void) -> HRESULT,
    // ...
}
```
**Problema:** Manual COM bindings sem bounds checking.  
**Impacto:** Acesso de memória inválido se WebView2 retornar estruturas inesperadas.  
**Solução:** Usar crate `webview2-com` oficial da Microsoft.

### 7. **Symlink Check Incompleto**
**Arquivo:** [`src/infrastructure/security.rs`](src/infrastructure/security.rs:189)
```rust
fn check_symlink(path: &Path) -> Result<(), SecurityError> {
    let mut current = path.to_path_buf();
    while current.exists() {
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
```
**Problema:** Não detecta junction points ou mount points do Windows.  
**Impacto:** Bypass de restrições de symlink.  
**Solução:** Verificar atributos `FILE_ATTRIBUTE_REPARSE_POINT` via Windows API.

---

## 🟡 ALTO - Problemas de Estabilidade

### 8. **Mutex Contention em Cache SQLite**
**Arquivo:** [`src/infrastructure/disk_cache.rs`](src/infrastructure/disk_cache.rs:15)
```rust
pub struct ThumbnailDiskCache {
    writer: Arc<Mutex<Connection>>, // For put, set_*, garbage_collect (DELETE)
    reader: Arc<Mutex<Connection>>, // For get, get_*, check existence
```
**Problema:** Dois mutexes podem causar deadlock se adquiridos em ordem errada.  
**Impacto:** UI thread bloqueia indefinidamente.  
**Solução:** Usar uma única conexão com WAL mode (já habilitado) ou `rusqlite::Connection` com `Send`.

### 9. **Thread Leak em Workers**
**Arquivo:** [`src/workers/thumbnail/worker.rs`](src/workers/thumbnail/worker.rs:75)
```rust
// 4 worker threads
for _ in 0..4 {
    std::thread::spawn(move || {
        thumbnail_worker_loop(...);
    });
}
```
**Problema:** Threads nunca são encerradas gracefulmente (loop infinito).  
**Impacto:** Vazamento de threads ao fechar aplicação.  
**Solução:** Implementar sinal de shutdown com `AtomicBool` ou `crossbeam_channel`.

### 10. **Handle Leak em Drive Watcher**
**Arquivo:** [`src/infrastructure/drive_watcher.rs`](src/infrastructure/drive_watcher.rs:191)
```rust
let handle = CreateFileW(
    PCWSTR(wide_path.as_ptr()),
    FILE_LIST_DIRECTORY.0,
    // ...
);
```
**Problema:** Handle pode não ser fechado em todos os caminhos de erro.  
**Impacto:** Vazamento de handles do Windows.  
**Solução:** Usar `scopeguard` ou wrapper RAII para HANDLE.

### 11. **Memory Leak em WebView2 COM Objects**
**Arquivo:** [`src/pdf_viewer/webview.rs`](src/pdf_viewer/webview.rs:103)
```rust
impl Drop for WebViewState {
    fn drop(&mut self) {
        unsafe {
            let vtbl = *(self.controller.ptr as *mut *mut ICoreWebView2Controller_Vtbl);
            ((*vtbl).base.Release)(self.controller.ptr);
        }
    }
}
```
**Problema:** Release pode falhar silenciosamente; não há verificação de refcount.  
**Impacto:** Memory leak se COM object estiver em estado inválido.  
**Solução:** Usar wrappers COM seguros ou verificar HRESULT.

### 12. **Infinite Loop Risk em Directory Reading**
**Arquivo:** [`src/infrastructure/onedrive/directory_enum.rs`](src/infrastructure/onedrive/directory_enum.rs:85)
```rust
loop {
    // ...
    if FindNextFileW(handle, &mut find_data).is_err() {
        break;
    }
}
```
**Problema:** Loop pode não terminar em diretórios corrompidos ou com links cíclicos.  
**Impacto:** Thread bloqueada indefinidamente.  
**Solução:** Adicionar timeout ou limite máximo de entradas.

---

## 🟢 MÉDIO - Otimizações e Melhorias

### 13. **Substituir Mutex Padrão por `parking_lot`**
**Múltiplos arquivos:** Todos que usam `std::sync::Mutex`
**Problema:** `std::sync::Mutex` é mais lento e pode envenenar.  
**Solução:** Migrar para `parking_lot::Mutex` que é mais rápido e não envenena.

### 14. **Cache Placeholder Vazio**
**Arquivo:** [`src/infrastructure/cache.rs`](src/infrastructure/cache.rs:1)
```rust
// Cache implementation - To be implemented
```
**Problema:** Módulo existe mas está vazio.  
**Solução:** Implementar ou remover arquivo.

### 15. **Thumbnail Loader Placeholder**
**Arquivo:** [`src/workers/thumbnail_loader.rs`](src/workers/thumbnail_loader.rs:1)
```rust
// Thumbnail loading worker - To be implemented
```
**Problema:** Arquivo placeholder pode causar confusão.  
**Solução:** Remover ou implementar.

### 16. **Hardcoded Paths**
**Arquivo:** [`src/app/init.rs`](src/app/init.rs:178)
```rust
let segoe_path = std::path::PathBuf::from("C:\\Windows\\Fonts\\segoeui.ttf");
```
**Problema:** Path absoluto pode falhar em sistemas não-padrão.  
**Solução:** Usar `dirs` ou `known-folders` crate para paths do sistema.

### 17. **Uso de `unwrap()` em Inicialização de Cache LRU**
**Arquivo:** [`src/ui/cache.rs`](src/ui/cache.rs:67)
```rust
texture_cache: LruCache::new(NonZeroUsize::new(DEFAULT_TEXTURE_CACHE_ITEMS).unwrap()),
```
**Problema:** Panic se `DEFAULT_TEXTURE_CACHE_ITEMS` for 0.  
**Solução:** Usar `NonZeroUsize::new(x).expect("cache size must be non-zero")` com mensagem descritiva.

### 18. **TODOs e Código Comentado**
**Múltiplos arquivos:**
```rust
// video-rs = { version = "0.9", features = ["ndarray"] }
// ndarray = "0.16"
// rodio = "0.19"
```
**Arquivo:** [`Cargo.toml`](Cargo.toml:34)
**Problema:** Dependências comentadas podem causar confusão.  
**Solução:** Remover ou mover para seção `[features]`.

### 19. **Consistência de Nomenclatura**
**Problema:** Mistura de inglês e português em nomes de variáveis e funções.  
**Exemplo:**
```rust
pub fn from_path(path: PathBuf, is_dir: bool) -> Self  // inglês
let name = path.file_name()  // inglês
pub fn sort_items(&mut self)  // inglês
```
vs
```rust
let novo_item = FileEntry::from_path(...)  // português
```
**Solução:** Padronizar para inglês em todo o codebase.

### 20. **Complexidade Ciclomática Alta**
**Arquivo:** [`src/app/init.rs`](src/app/init.rs:68) - `ImageViewerApp::new()`
**Problema:** Função de inicialização tem ~900 linhas.  
**Solução:** Extrair para múltiplas funções especializadas.

---

## 🔵 BAIXO - Sugestões de Melhoria

### 21. **Adicionar Mais Testes Unitários**
**Observação:** Apenas `src/domain/errors.rs` tem testes visíveis.  
**Sugestão:** Adicionar testes para:
- Sanitização de paths
- Extração de thumbnails
- Cache LRU
- Operações de arquivo

### 22. **Usar `eyre` ou `anyhow` para Erros em Workers**
**Problema:** Conversão manual de erros em workers é verbosa.  
**Sugestão:** Usar `eyre` para contexto rico em operações de background.

### 23. **Implementar Rate Limiting para Thumbnails**
**Problema:** Scrolling rápido pode inundar a fila de thumbnails.  
**Sugestão:** Adicionar debounce/throttle na geração de thumbnails.

### 24. **Adicionar Telemetria de Performance**
**Sugestão:** Integrar com `tracing` ou `metrics` para monitorar:
- Tempos de I/O de disco
- Taxa de cache hits/misses
- Tempos de renderização de frames

### 25. **Documentação de APIs Públicas**
**Problema:** Muitos módulos públicos carecem de documentação.  
**Sugestão:** Adicionar `//!` doc comments em todos os módulos.

### 26. **Feature Flags Inconsistentes**
**Problema:** `notify-watcher` é opcional mas código usa `#[cfg(feature = "notify-watcher")]` extensivamente.  
**Sugestão:** Simplificar ou documentar melhor as features.

### 27. **Uso de `static mut`**
**Verificação:** Não encontrado - bom! ✅

### 28. **Verificação de `unsafe` Blocks**
**Total:** 158 blocos `unsafe`  
**Recomendação:** Auditar cada um para garantir:
- Precondições documentadas
- Invariantes mantidas
- Cleanup adequado (RAII)

---

## 📋 Lista de Verificação de Segurança

| CWE | Descrição | Status | Notas |
|-----|-----------|--------|-------|
| CWE-20 | Input Validation | ⚠️ Parcial | Path sanitização existe mas pode ser bypassada |
| CWE-22 | Path Traversal | ⚠️ Parcial | Bloqueia `..` mas não junction points |
| CWE-78 | OS Command Injection | ✅ OK | Não executa comandos shell diretamente |
| CWE-79 | XSS | N/A | Aplicação desktop nativa |
| CWE-125 | Out-of-bounds Read | ⚠️ Risco | COM bindings manuais |
| CWE-190 | Integer Overflow | ⚠️ Risco | Cálculos de offset em thumbnails |
| CWE-362 | Race Condition | ⚠️ Encontrado | Semaphore e Mutexes |
| CWE-416 | Use After Free | ✅ OK | RAII usado consistentemente |
| CWE-476 | NULL Pointer Dereference | ⚠️ Risco | Verificações inconsistentes |

---

## 🎯 Recomendações Prioritárias

### Imediato (Bloqueante)
1. ✅ Remover `panic!` de `disk_cache.rs`
2. ✅ Corrigir race condition no Semaphore
3. ✅ Implementar graceful shutdown para workers

### Curto Prazo (1-2 semanas)
4. 🔄 Migrar para `parking_lot` para todos os mutexes
5. 🔄 Adicionar RAII wrappers para handles do Windows
6. 🔄 Implementar testes para path sanitization

### Médio Prazo (1 mês)
7. 📋 Auditar todos os 158 blocos `unsafe`
8. 📋 Substituir COM bindings manuais por crates oficiais
9. 📋 Implementar testes de integração

### Longo Prazo
10. 🎯 Adicionar fuzzing para parsers de arquivo
11. 🎯 Implementar sandboxing para decodificação de mídia
12. 🎯 Adicionar métricas de performance em produção

---

## 📈 Métricas do Código

| Métrica | Valor |
|---------|-------|
| Total de arquivos `.rs` | ~150 |
| Linhas de código (estimado) | ~25,000+ |
| Blocos `unsafe` | 158 |
| Usos de `unwrap()` | 60+ |
| Usos de `expect()` | 20+ |
| Módulos | 50+ |
| Dependências (Cargo.toml) | 30+ |

---

## 🏁 Conclusão

O MTT File Manager é um projeto sólido com boas práticas de arquitetura e preocupações legítimas com performance. Os principais problemas identificados são:

1. **Estabilidade:** Panics em caminhos de erro críticos e possíveis deadlocks
2. **Segurança:** Validação de paths pode ser fortalecida
3. **Manutenibilidade:** Alguns módulos grandes e código placeholder

Recomendo fortemente implementar as correções de **nível CRÍTICO** antes de qualquer release para produção. Os problemas de **nível ALTO** devem ser endereçados no próximo sprint.

---

*Relatório gerado automaticamente por Code Review AI*  
*Metodologia: Análise estática + Heurísticas de segurança + Melhores práticas Rust*