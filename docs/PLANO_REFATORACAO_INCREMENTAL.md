# Plano de Refatoração Incremental - MTT File Manager

> **Objetivo:** Aplicar módulos já existentes de forma gradual e segura, mantendo a aplicação sempre funcional.

---

## 📋 Progresso Atual (30/12/2025)

### ✅ Sprint 1: Infrastructure - CORREÇÃO DE IMPORTS

**Status:** Concluído com sucesso  
**Problema Resolvido:** Os módulos `windows/*.rs` estavam importando de `crate::infrastructure::windows_api` que não existia como módulo válido.

**Mudanças Realizadas:**

1. **Correção de Imports em Todos os Módulos Windows:**
   - `src/infrastructure/windows/bitmap_conversion.rs`
   - `src/infrastructure/windows/drives.rs`
   - `src/infrastructure/windows/file_system.rs`
   - `src/infrastructure/windows/icons.rs`
   - `src/infrastructure/windows/shell_operations.rs`
   - `src/infrastructure/windows/system_info.rs`

2. **Atualização de Consumidores:**
   - `src/workers/batch_thumbnail_loader.rs` - Corrigido import de `windows::Win32::System::Com`
   - `src/ui/views/common.rs` - Alterado de `windows_api` para `windows`

3. **Resultado:**
   - ✅ Código compila sem erros: `cargo build --release`
   - ✅ Aplicação funciona normalmente: `cargo run --release`
   - ✅ Módulos `windows` agora importam diretamente da crate `windows`

**Estrutura Atualizada:**
```
src/infrastructure/
├── windows/
│   ├── bitmap_conversion.rs ✅
│   ├── drives.rs ✅
│   ├── file_system.rs ✅
│   ├── formatting.rs ✅
│   ├── icons.rs ✅
│   ├── shell_operations.rs ✅
│   └── system_info.rs ✅
├── cache.rs
└── watcher.rs
```

**Arquivo Removido:** `windows_api.rs` (vazio, não mais necessário)

---

## 📋 Estado Original (para referência)

### Estrutura Existente

```
src/
├── main.rs (136KB) ⚠️ MONOLÍTICO - Contém toda a lógica
├── lib.rs (119 bytes) - Entry point da biblioteca
├── application/      ✅ Módulos prontos mas não usados
│   ├── state.rs
│   ├── clipboard.rs
│   ├── context_menu.rs
│   ├── navigation.rs
│   ├── renaming.rs
│   └── watcher.rs
├── domain/          ✅ Usado parcialmente
│   ├── file_entry.rs
│   └── thumbnail.rs
├── infrastructure/  ✅ Módulos prontos mas não usados
│   └── windows_api/ (13 arquivos)
├── ui/              ✅ Módulos prontos mas não usados
│   ├── app.rs (stub)
│   ├── cache.rs
│   ├── navigation.rs
│   ├── operations.rs
│   └── views/ (grid_view.rs, list_view.rs, computer_view.rs)
└── workers/         ✅ Módulos prontos mas não usados
    └── (4 arquivos)
```

### Problema Identificado Anteriormente

As views refatoradas (`grid_view.rs`, `list_view.rs`) esperam:
```rust
self.is_computer_view  // ❌ Campo direto
self.thumbnail_size    // ❌ Campo direto
```

Mas a estrutura modular usa:
```rust
self.state.is_computer_view  // ✅ Nested em state
self.state.thumbnail_size    // ✅ Nested em state
```

**Resultado:** Incompatibilidade estrutural → 76 erros de compilação.

---

## 🎯 Estratégia: Refatoração Incremental por "Slices Verticais"

### Princípios

1. **Sempre Compilável** - Cada passo compila sem erros
2. **Sempre Testável** - Funcionalidade testada após cada passo
3. **Commits Frequentes** - Um commit por módulo integrado
4. **Rollback Fácil** - `git revert` para voltar se necessário

### Ordem de Integração

```
Sprint 1: Infrastructure (baixo risco)
  └─> Windows APIs isoladas, sem dependências de UI

Sprint 2: Workers (médio risco)  
  └─> Threads em background, testáveis separadamente

Sprint 3: Application State (alto risco)
  └─> Requer mudança de estrutura de dados

Sprint 4: UI Components (alto risco)
  └─> Requer adaptação de views existentes
```

---

## 📝 Sprint 1: Infrastructure (Baixo Risco)

### Objetivo
Mover funções de Windows API de `main.rs` para `infrastructure/windows_api/`.

### Pré-requisitos
- [ ] Branch `Refatoracao-Complete` atualizado
- [ ] Código compilando: `cargo build --release`
- [ ] Aplicação testada e funcionando

### Passo 1.1: Mover `extract_computer_icon()`

**Arquivo:** `src/infrastructure/windows_api/icons.rs`

1. **Copiar** função de `main.rs` (linhas 62-111):
   ```rust
   pub fn extract_computer_icon(size: IconSize) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>>
   ```

2. **Adicionar export** em `src/infrastructure/windows_api/mod.rs`:
   ```rust
   pub use icons::extract_computer_icon;
   ```

3. **Substituir** em `main.rs`:
   ```rust
   // ANTES
   fn extract_computer_icon(size: IconSize) -> ...
   
   // DEPOIS
   use mtt_file_manager::infrastructure::windows_api::extract_computer_icon;
   ```

4. **Testar**:
   ```bash
   cargo build --release
   cargo run --release
   ```

5. **Verificar**: Ícone "Este Computador" aparece na sidebar

6. **Commit**:
   ```bash
   git add -A
   git commit -m "refactor: move extract_computer_icon to infrastructure/windows_api"
   ```

### Passo 1.2: Mover `extract_drive_icon()`

**Repetir processo do Passo 1.1 para:**
- Função: `extract_drive_icon()` (linhas 772-825 em main.rs)
- Arquivo destino: `src/infrastructure/windows_api/icons.rs`

### Passo 1.3: Mover funções de thumbnail

**Mover para `src/infrastructure/windows_api/thumbnails.rs`:**
- `extract_windows_thumbnail()`
- `hbitmap_to_rgba()`
- `create_error_placeholder()`

### Checklist Sprint 1

- [ ] Passo 1.1 executado e testado
- [ ] Passo 1.2 executado e testado
- [ ] Passo 1.3 executado e testado
- [ ] Código compila sem warnings
- [ ] Aplicação funciona normalmente
- [ ] Commit final: "refactor(sprint1): move all Windows API functions to infrastructure"

**Risco:** 🟢 Baixo (funções isoladas, sem dependências de estado)

---

## 📝 Sprint 2: Workers (Médio Risco)

### Objetivo
Extrair lógica de workers para `src/workers/`.

### Passo 2.1: Extrair Thumbnail Worker

1. **Criar** `src/workers/thumbnail_worker.rs`:
   ```rust
   pub fn spawn_thumbnail_workers(
       shared_rx: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
       tx: Sender<ThumbnailData>,
       ctx: egui::Context,
       gen_tracker: Arc<AtomicUsize>,
   ) {
       for _ in 0..4 {
           let rx = shared_rx.clone();
           let tx = tx.clone();
           let gen = gen_tracker.clone();
           let ctx = ctx.clone();
           
           std::thread::spawn(move || {
               thumbnail_worker_loop(rx, tx, ctx, gen);
           });
       }
   }
   
   fn thumbnail_worker_loop(...) {
       // Código do loop (linhas 253-292 de main.rs)
   }
   ```

2. **Substituir** em `main.rs::new()`:
   ```rust
   // ANTES
   for _ in 0..4 { std::thread::spawn(...) }
   
   // DEPOIS
   use mtt_file_manager::workers::spawn_thumbnail_workers;
   spawn_thumbnail_workers(shared_req_rx, img_tx, ctx.clone(), shared_gen.clone());
   ```

3. **Testar**: Thumbnails carregam normalmente

4. **Commit**: "refactor: extract thumbnail worker to workers module"

### Passo 2.2: Extrair Cover Worker

Similar ao 2.1, para a lógica de folder covers (linhas 234-243).

### Checklist Sprint 2

- [ ] Thumbnail workers extraídos
- [ ] Cover worker extraído
- [ ] Thumbnails carregam corretamente
- [ ] Capas de pasta funcionam
- [ ] Commit final

**Risco:** 🟡 Médio (threads, mas testáveis isoladamente)

---

## 📝 Sprint 3: Application State (Alto Risco - APENAS SE NECESSÁRIO)

> ⚠️ **AVISO:** Este sprint muda a estrutura de dados. Só execute se estiver confortável com mudanças profundas.

### Pre-Decisão Estratégica

**Opção A (Recomendada):** Manter estrutura atual de `main.rs`
- ✅ Código funciona
- ✅ Zero risco
- ❌ Campos não agrupados em `state`

**Opção B (Avançada):** Migrar para `self.state.*`
- ✅ Arquitetura mais limpa
- ❌ Alto risco de quebra
- ❌ Requer adaptação de TODAS as views

**Recomendação:** SKIP Sprint 3 por enquanto. Focar em Sprints 1 e 2.

---

## 📝 Sprint 4: UI Components (Futuro)

### Objetivo
Extrair componentes de UI para `ui/components/`.

### Candidatos (após Sprints 1-2)

1. **Top Bar** (linhas 2559-2686)
   - Navegação (back/forward/up)
   - Busca
   - Barra de endereço

2. **Sidebar** (linhas 2688-2832)
   - "Este Computador" header
   - Lista de drives

3. **Status Bar**
   - Contadores de itens
   - Modo de visualização

### Estratégia

Cada componente:
1. Extrair para arquivo separado em `ui/components/`
2. Criar função `render_*()` que recebe `&mut ImageViewerApp`
3. Substituir código inline por chamada à função
4. Testar isoladamente

---

## 🧪 Protocolo de Testes

### Após Cada Passo

```bash
# 1. Compilar
cargo build --release

# 2. Executar
cargo run --release

# 3. Testar manualmente
Navegação:
  ✓ Voltar/Avançar funciona
  ✓ Subir nível funciona
  ✓ Barra de endereço funciona

Este Computador:
  ✓ Clicar em "Este Computador" mostra drives
  ✓ Clicar em drive navega para ele

Thumbnails:
  ✓ Imagens carregam
  ✓ Pastas mostram capas

Operações:
  ✓ Botão direito abre menu
  ✓ Copiar/Colar funciona
  ✓ Renomear funciona
```

### Se Algo Quebrar

```bash
# Reverter último commit
git revert HEAD

# OU voltar para commit anterior
git reset --hard HEAD~1

# OU trocar para branch seguro
git checkout Refatoracao-Working
```

---

## 📊 Métricas de Sucesso

### Por Sprint

| Sprint | Arquivos Afetados | Linhas Movidas | Redução main.rs | Risco |
|--------|------------------|----------------|-----------------|-------|
| 1      | ~5               | ~500           | ~3%             | 🟢    |
| 2      | ~3               | ~200           | ~1.5%           | 🟡    |
| 3      | ~10              | ~1000          | ~7%             | 🔴    |
| 4      | ~8               | ~800           | ~6%             | 🟡    |

### Objetivo Final (Longo Prazo)

- `main.rs`: < 2000 linhas (de 3134)
- Todos arquivos: < 300 linhas (regra .cursorrules)
- Cobertura de testes: > 50%

---

## 🚨 Regras de Segurança

### NUNCA quebrar estas regras:

1. **Sempre compile antes de commit**
   ```bash
   cargo build --release || echo "❌ NÃO COMMITAR"
   ```

2. **Teste aplicação antes de commit**
   - Abrir aplicação
   - Navegar entre pastas
   - Testar funcionalidade modificada

3. **Um commit por módulo**
   - Facilita rollback
   - Histórico limpo

4. **Mensagem de commit descritiva**
   ```
   refactor: move extract_computer_icon to infrastructure
   
   - Moved function from main.rs to infrastructure/windows_api/icons.rs
   - Added export in mod.rs
   - Tested: Computer icon displays correctly
   ```

5. **Branch protegido**
   - `Refatoracao-Working` = NUNCA tocar
   - `Refatoracao-Complete` = Trabalho ativo
   - `main` = Versão original

---

## 📅 Cronograma Sugerido

### Semana 1: Sprint 1 (Infrastructure)
- Dia 1-2: Mover funções de ícones
- Dia 3-4: Mover funções de thumbnail
- Dia 5: Testes e ajustes

### Semana 2: Sprint 2 (Workers)
- Dia 1-3: Extrair workers
- Dia 4-5: Testes e validação

### Semana 3: Revisão
- Avaliar necessidade de Sprint 3
- Planejar Sprint 4

---

## 🎓 Lições do Erro Anterior

### O que NÃO fazer:

❌ **Big Bang Refactoring** - Mudar tudo de uma vez  
❌ **Estrutura incompatível** - Views esperando `self.campo` mas usando `self.state.campo`  
❌ **Commits sem testes** - Código quebrado no repositório  
❌ **Mudanças sem protocolo de rollback**

### O que FAZER:

✅ **Baby Steps** - Um módulo por vez  
✅ **Sempre funcional** - Compilar e testar a cada passo  
✅ **Commits frequentes** - Fácil de reverter  
✅ **Branch de segurança** - Backup sempre disponível

---

## 📞 Suporte

### Se tiver dúvidas:

1. Revisar este plano
2. Testar em branch separado primeiro
3. Fazer rollback se necessário
4. Documentar problemas encontrados

### Arquivos de Referência

- `src/main.rs` (linhas 1-3134) - Código funcional atual
- `src/ui/views/grid_view.rs` - Exemplo de view modular
- `src/application/state.rs` - Exemplo de state management

---

**Última atualização:** 2025-12-30  
**Versão:** 1.0  
**Autor:** Equipe MTT File Manager
