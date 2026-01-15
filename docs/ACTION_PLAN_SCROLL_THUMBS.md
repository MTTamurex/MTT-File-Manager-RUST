# Plano de Ação – Thumbnails & Scroll Suave

Objetivo: acelerar carregamento de thumbnails e melhorar fluidez de scroll em lista/grid sem alterar lógicas de negócio.

## Prioridade Alta
6) [DONE] Fila com prioridade para thumbnails visíveis
   - Implementado `PriorityThumbnailQueue` com LIFO (High) e FIFO (Low).
   - Métodos `request_thumbnail_load` (High) e `request_thumbnail_prefetch` (Low).

2) [DONE] Throttle de repaints nos workers de imagem
   - Implementado `throttle_repaint` em `thumbnail_worker` e `folder_preview_worker`.
   - Limita wakeups da thread UI para ~30fps durante carga pesada.

3) Evitar re-decode em navegação rápida
   - Manter LRU de caminhos recentes com timestamp; se decode recente existir em cache disco/mem, não reenfileirar antes de expirar.

## Prioridade Média
4) [DONE-Grid] Culling agressivo em lista/grid
   - `grid_view.rs` usa virtualização com cálculo de linhas visíveis (`visible_rows_range`).
   - Prefetch logic também usa esse range.

5) Cache de layout de texto
   - Cachear `TextLayoutJob` (nome/tamanho/data) por item, invalidando apenas quando muda fonte/largura.
   - Evita recalcular layout em cada frame durante scroll.

6) [DONE] Texturas em buckets fixos
   - Implementado `resize_to_bucket` (128/256/512/1024).
   - Otimiza uso de RAM e Upload para GPU.

## Prioridade Baixa
7) Smoothing de scroll e debounce de hover
   - Acumular delta de wheel com clamp/easing leve para evitar saltos grandes.
   - Debounce de tooltip/hover (150–200 ms) enquanto scroll está ativo para reduzir trabalho por frame.

## Verificação
- Medir FPS e tempo de frame ao rolar pasta grande (10k itens).
- Confirmar ausência de repaints em storm em carregamento inicial.
- Navegação rápida (PgDown) não re-decoda thumbs já em cache recente.
