# Plano de Implementação - Correção de Travamentos

## Problema Identificado

O aplicativo trava após ser minimizado por um tempo e depois restaurado. O erro `0xcfffffff` indica problema de recurso/concorrência.

## Causa Raiz

As funções de timeout em `onedrive.rs` criam threads temporárias (`std::thread::spawn`) para cada operação de I/O. Quando muitas operações são iniciadas enquanto o app está minimizado:

1. **Thread leak** - threads acumulam sem serem joined
2. **Resource exhaustion** - esgotamento de handles/threads do sistema  
3. **Deadlock** - múltiplas threads competindo por recursos

## Solução Proposta

### Fase 1: Limitar Threads de Timeout (CRÍTICO)

**Arquivo:** `src/infrastructure/onedrive.rs`

```rust
// Adicionar no início do arquivo
use std::sync::{Arc, Semaphore};
use std::sync::atomic::{AtomicU64, Ordering};

// Semáforo global para limitar threads de timeout concorrentes
static TIMEOUT_SEMAPHORE: once_cell::sync::Lazy<Arc<Semaphore>> = 
    once_cell::sync::Lazy::new(|| Arc::new(Semaphore::new(4)));

// Contador de threads ativas para monitoramento
static ACTIVE_TIMEOUT_THREADS: AtomicU64 = AtomicU64::new(0);
```

Modificar as funções `metadata_with_timeout`, `exists_with_timeout`, `read_directory_with_timeout` para:

1. Adquirir permissão do semáforo antes de spawnar thread
2. Liberar semáforo após timeout ou conclusão
3. Se semáforo não disponível, retornar Timeout imediatamente

### Fase 2: Cancelar Operações ao Minimizar

**Arquivo:** `src/ui/app/lifecycle.rs`

```rust
// Adicionar flag global de "app minimizado"
static APP_MINIMIZED: AtomicBool = AtomicBool::new(false);

pub fn set_minimized(minimized: bool) {
    APP_MINIMIZED.store(minimized, Ordering::SeqCst);
    if minimized {
        eprintln!("[LIFECYCLE] App minimized - canceling pending operations");
        // Incrementar generation para invalidar operações pendentes
    }
}
```

**Arquivo:** `src/infrastructure/onedrive.rs`

Verificar `APP_MINIMIZED` antes de iniciar operação de I/O, retornar Timeout se minimizado.

### Fase 3: Timeout mais agressivo

Reduzir timeouts quando app está minimizado:

```rust
const ONEDRIVE_METADATA_TIMEOUT_MS: u64 = 100;
const ONEDRIVE_METADATA_TIMEOUT_MINIMIZED_MS: u64 = 50;
```

### Fase 4: Logging e Monitoramento

Adicionar logs para rastrear:
- Número de threads de timeout ativas
- Operações canceladas por minimização
- Timeouts ocorridos

## Implementação Passo a Passo

### Passo 1: Adicionar dependência no Cargo.toml
```toml
[dependencies]
once_cell = "1.19"
```

### Passo 2: Modificar onedrive.rs

1. Adicionar imports e variáveis globais
2. Criar função `try_timeout_operation` com semáforo
3. Modificar `metadata_with_timeout` para usar semáforo
4. Modificar `exists_with_timeout` para usar semáforo  
5. Modificar `read_directory_with_timeout` para usar semáforo

### Passo 3: Modificar lifecycle.rs

1. Adicionar flag `APP_MINIMIZED`
2. Chamar `set_minimized(true)` quando detectar minimização
3. Chamar `set_minimized(false)` quando restaurar

### Passo 4: Testar

1. Build: `cargo build --release`
2. Testar minimização por 5 minutos
3. Verificar logs de threads ativas
4. Confirmar que não há mais travamentos

## Código de Exemplo - Função com Semáforo

```rust
pub fn metadata_with_timeout(
    path: &Path,
    timeout_ms: u64,
) -> IoTimeoutResult<std::fs::Metadata> {
    // Check if app is minimized - use shorter timeout
    let timeout_ms = if is_app_minimized() {
        timeout_ms / 2
    } else {
        timeout_ms
    };

    // Try to acquire semaphore permit (max 4 concurrent)
    let permit = match TIMEOUT_SEMAPHORE.clone().try_acquire() {
        Ok(p) => p,
        Err(_) => {
            eprintln!("[ONEDRIVE] Too many concurrent timeout operations");
            return IoTimeoutResult::Timeout;
        }
    };

    let active = ACTIVE_TIMEOUT_THREADS.fetch_add(1, Ordering::SeqCst) + 1;
    eprintln!("[ONEDRIVE] Active timeout threads: {}", active);

    // ... resto da implementação ...

    // Permit is automatically released when dropped
    drop(permit);
    ACTIVE_TIMEOUT_THREADS.fetch_sub(1, Ordering::SeqCst);
    
    result
}
```

## Benefícios

1. **Previne thread exhaustion** - máximo 4 threads de timeout simultâneas
2. **Evita deadlock** - semáforo garante ordem de execução
3. **Responsivo ao minimizar** - cancela/operações pendentes
4. **Monitorável** - logs mostram comportamento em tempo real

## Tempo Estimado

- Implementação: 30 minutos
- Testes: 15 minutos
- Total: 45 minutos

## Próximos Passos

1. Aprovar este plano
2. Implementar Fase 1 (mais crítica)
3. Testar
4. Se necessário, implementar Fases 2-4
