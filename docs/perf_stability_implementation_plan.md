# Plano de Implementação — Performance e Estabilidade

## Contexto

Este plano operacionaliza os achados do code review com foco em ganhos reais de performance e estabilidade, sem escopo extra.

## Escopo aprovado

1. Corrigir medição de tempo do batch adaptativo no fast path.
2. Remover verificação síncrona potencialmente bloqueante na resolução de capa de pasta em caminho acionado pela UI.
3. Eliminar probe síncrono de metadata no painel de resultados da busca global.
4. Validar compilação com `cargo check`.

## Checklist de execução

- [x] Etapa 1 — Corrigir medição de tempo do AdaptiveBatch no fast path
- [x] Etapa 2 — Remover verificação bloqueante de existência de capa de pasta na UI path
- [x] Etapa 3 — Eliminar probe síncrono de metadata no painel de resultados da busca global
- [x] Etapa 4 — Validar compilação (`cargo check`)
- [x] Etapa 5 — Documentar conclusão de cada etapa no plano

## Registro de execução por etapa

### Etapa 1 — AdaptiveBatch no fast path
- **Status:** Concluída
- **Arquivos alvo:** `src/app/operations/folder_loading/load_pipeline/fast_paths.rs`
- **Resumo:** Substituída a medição incorreta baseada em `Instant::now().elapsed()` por medição real de duração por chunk usando `batch_start.elapsed()`, reiniciando `batch_start` no início de cada iteração dos loops de envio no fast path.

### Etapa 2 — Cover validation sem I/O bloqueante na UI path
- **Status:** Concluída
- **Arquivos alvo:** `src/app/operations/folder_loading/folder_scan.rs`
- **Resumo:** Substituída a verificação síncrona `path.exists()` por `onedrive::fast_path_exists(path)` na validação de capa em cache, mantendo semântica de existência e reduzindo risco de bloqueio em caminhos cloud/virtualizados.

### Etapa 3 — Busca global sem metadata síncrona no render
- **Status:** Concluída
- **Arquivos alvo:** `src/ui/global_search_overlay/results_panel.rs`
- **Resumo:** Removido o probe síncrono `std::fs::metadata` do caminho de render da lista de resultados; a resolução de tamanho agora utiliza somente `size` vindo da busca e cache local (`size_cache`) com fallback imediato para `None` (render de `-`) sem I/O bloqueante.

### Etapa 4 — Verificação de compilação
- **Status:** Concluída
- **Comando:** `cargo check`
- **Resumo:** `cargo check` executado com sucesso (exit code 0), sem erros de compilação.

### Etapa 5 — Documentação incremental do progresso
- **Status:** Concluída
- **Resumo:** Cada etapa concluída foi registrada imediatamente neste plano, com status, arquivos-alvo e resultado objetivo.
