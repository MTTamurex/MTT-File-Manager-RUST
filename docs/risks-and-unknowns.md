# Riscos e Pontos Desconhecidos - MTT File Manager

## 1. Código Incompleto ou Placeholder

### Arquivos Vazios/Stub

| Arquivo | Tamanho | Status |
|---------|---------|--------|
| `infrastructure/cache.rs` | 45 bytes | Apenas `// TODO` ou vazio |
| `infrastructure/watcher.rs` | 44 bytes | Placeholder |
| `workers/folder_scanner.rs` | 47 bytes | Stub não implementado |
| `workers/thumbnail_loader.rs` | 49 bytes | Stub não implementado |

**Impacto:** Esses módulos provavelmente foram planejados mas não implementados. A funcionalidade pode estar em outros locais ou pendente.

---

## 2. Código Morto / Temporário

### Arquivos para Remoção

| Arquivo | Evidência | Recomendação |
|---------|-----------|--------------|
| `temp_snippet_shell_ops.rs` | Prefixo `temp_` | **REMOVER** |
| `errors.txt` | Arquivo de debug | Ignorar/Remover |
| `hd_perf_check.txt` | Notas de performance | Mover para docs ou remover |

### Dependências Comentadas

```toml
# Em Cargo.toml - Experimentos anteriores
# video-rs = { version = "0.9", features = ["ndarray"] }
# ndarray = "0.16"
# rodio = "0.19"
```

**Status:** Experimentos abandonados de integração de vídeo/áudio alternativos

---

## 3. Arquivos Excepcionalmente Grandes

| Arquivo | Tamanho | Preocupação |
|---------|---------|-------------|
| `ui/preview_panel.rs` | 59KB / ~1500 linhas | Candidato a refatoração |
| `app/operations/ui_rendering.rs` | 38KB | Alta complexidade |
| `workers/thumbnail_worker.rs` | 33KB / 925 linhas | Justificado pela complexidade |
| `ui/views/list_view.rs` | 34KB | Muita lógica inline |

**Risco:** Arquivos grandes são difíceis de manter e entender. `preview_panel.rs` em particular combina múltiplas responsabilidades.

---

## 4. Dependências Pouco Claras

### Dependência de mpv

- **Arquivo:** `mpv.lib` na raiz (173KB)
- **Runtime:** `libmpv-2.dll` necessário
- **Status:** Não há documentação sobre versão ou obtenção
- **Risco:** Pode quebrar com atualizações do mpv

### Dependência de WebView2

- **Uso:** PDF Viewer
- **Status:** Assume que Edge WebView2 está instalado
- **Risco:** Falha silenciosa em sistemas sem WebView2

### Fontes do Sistema

- **Hardcoded:** `C:\Windows\Fonts\segoeui.ttf`
- **Risco:** Falha em instalações Windows não-padrão ou outros idiomas

---

## 5. Funcionalidades Parcialmente Implementadas

### OneDrive Sync Status

- **Arquivo:** `infrastructure/onedrive.rs` (5.7KB)
- **Arquivo:** `domain/file_entry.rs` (enum `SyncStatus`)
- **Status:** Estrutura existe, implementação de detecção pode estar incompleta
- **Evidência:** Enum tem 5 estados, uso no código não é consistente

### NVIDIA VSR

- **Arquivo:** `ui/components/mpv_preview.rs`
- **Funções:** `enable_nvidia_vsr()`, `disable_vsr()`
- **Status:** Implementado mas sem UI para ativar
- **Evidência:** Funcionalidade existe mas não é acessível ao usuário

### Warmup de Shell Extensions

- **Arquivo:** `infrastructure/windows/native_menu.rs`
- **Função:** `warmup_shell_extensions()`
- **Status:** Documentado mas não claro quando/se é chamado
- **Comentário no código:** "Call this on app startup"

---

## 6. Trechos Frágeis / Alto Acoplamento

### `ImageViewerApp` State Struct

- **Arquivo:** `app/state.rs`
- **Problema:** ~50 campos em uma única struct
- **Risco:** Mudanças afetam toda a aplicação

### Comunicação via Channels

- **Localização:** Múltiplos módulos
- **Tipos de channel:**
  - Thumbnail results
  - Folder content
  - File operations
  - Recycle bin items
- **Risco:** Erros de runtime se channels são usados incorretamente

### Window Subclass

- **Arquivo:** `infrastructure/windows/window_subclass.rs`
- **Uso:** Custom borderless window
- **Risco:** `unsafe` code, difícil de debugar, pode causar crashes

---

## 7. Código `unsafe`

### Localizações de `unsafe`

| Arquivo | Uso |
|---------|-----|
| `infrastructure/windows/*.rs` | Windows API calls |
| `workers/thumbnail_worker.rs` | Media Foundation |
| `infrastructure/windows_clipboard.rs` | CF_HDROP handling |
| `ui/components/mpv_preview.rs` | HWND manipulation |

**Documentação:** Alguns blocos têm comentários `// SAFETY:`, outros não

**Risco:** Memory safety depende de invariantes não verificadas

---

## 8. Tratamento de Erros

### Uso de `unwrap()` e `expect()`

⚠️ **Potenciais pontos de panic:**
- Provavelmente existem em código de inicialização
- Macros `safe_unwrap!` e `safe_expect!` existem mas uso não é universal

### Erros Silenciados

```rust
// Padrão observado em vários arquivos
if let Err(e) = operation() {
    eprintln!("Error: {}", e);
    // Continua execução
}
```

**Risco:** Falhas podem passar despercebidas

---

## 9. Partes Impossíveis de Entender Sem Contexto

### Números Mágicos

```rust
// Em thumbnail_worker.rs
fn get_bucket_size(req_size: u32) -> u32 {
    match req_size {
        0..=64 => 64,
        65..=128 => 128,
        129..=256 => 256,
        _ => 512,
    }
}
```
**Contexto necessário:** Razão para esses tamanhos específicos

### Constantes de Codec

- **Arquivo:** `infrastructure/windows/codec_registry.rs` (22KB)
- **Conteúdo:** Mapeamento de GUIDs para nomes de codecs
- **Fonte:** Provavelmente documentação Microsoft ou engenharia reversa

### PropertyKeys

- **Arquivo:** `infrastructure/windows/metadata/property_keys.rs`
- **Conteúdo:** GUIDs de propriedades do Shell
- **Fonte:** Windows SDK headers

---

## 10. Testes

### Estado de Testes

| Tipo | Status |
|------|--------|
| Unit tests | **MÍNIMOS** - Apenas em `domain/errors.rs` |
| Integration tests | **INEXISTENTES** |
| E2E tests | **INEXISTENTES** |

**Risco:** Sem cobertura de testes, regressões são prováveis

---

## 11. Internacionalização

### Strings Hardcoded

```rust
// Exemplos encontrados no código
"Pasta"
"Arquivo"
"Arquivo ZIP"
"Este Computador"
```

**Status:** Todas as strings UI estão em Português (BR) hardcoded
**Risco:** Internacionalização requer refatoração significativa

---

## 12. Segurança

### Path Traversal

- **Arquivo:** `infrastructure/security.rs` (12KB)
- **Status:** Existe módulo de segurança
- **Verificação necessária:** Confirmar que todas as entradas de path são sanitizadas

### Execução de Comandos

- **Uso de `ShellExecuteW`:** Abre arquivos com aplicação padrão
- **Risco:** Se path for manipulado, pode executar código arbitrário

---

## Resumo de Criticidade

| Categoria | Itens | Impacto |
|-----------|-------|---------|
| **ALTO** | Arquivos grandes, estado centralizado, código unsafe | Manutenibilidade |
| **MÉDIO** | Dependências externas, funcionalidades incompletas | Estabilidade |
| **BAIXO** | Código morto, arquivos temp, i18n | Limpeza |

---

## Próximos Passos Recomendados

1. **Remover código morto** - `temp_snippet_shell_ops.rs`, dependências comentadas
2. **Refatorar arquivos grandes** - Especialmente `preview_panel.rs`
3. **Adicionar testes** - Pelo menos para módulos críticos
4. **Documentar `unsafe`** - Adicionar comentários SAFETY onde faltam
5. **Verificar tratamento de erros** - Auditar uso de `unwrap()`
6. **Documentar dependências externas** - Criar instruções para mpv/WebView2
