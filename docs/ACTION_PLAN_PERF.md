# Plano de Ação – Performance e Otimização

Foco: resolver gargalos de CPU/IO/repaint antes de ajustes estéticos.

## Prioridade Alta (tratar primeiro)
1) Inicialização assíncrona do WebView/FFprobe
   - Mover `start_video_server` e `probe_video_codecs_internal` para thread curta; retornar `(port, codec_info)` via channel; mostrar spinner até concluir.
   - Evita jank/freeze na UI ao abrir vídeos pesados ou em rede.

2) Encerramento correto do servidor de vídeo
   - Implementar `Drop` em `WebviewPreview` enviando shutdown para o listener e ocultando o WebView.
   - Prevê vazamento de threads/portas ao trocar de arquivo/aba.

3) Throttle de repaints em workers
   - Substituir `ctx.request_repaint()` por throttle (>=16–33ms) em `thumbnail_worker` e `folder_preview_worker`; opcional: disparar só ao final de cada lote.
   - Reduz picos de CPU na thread de UI durante carregamentos grandes.

## Prioridade Média
4) Throttle de IPC/repintar no player WebView
   - Ajustar `request_repaint_after` para 120ms quando tocando e 600ms quando pausado/oculto.
   - No JS embutido, reduzir frequência do `postMessage` quando pausado.
   - Diminui parse de JSON e repaints inúteis em abas ocultas.

5) Limitar concorrência de decodificação sem busy-wait
   - Trocar loop de espera (`while active_decodes... sleep`) por `Condvar` ou canal com capacidade para sinalizar slots livres.
   - Menos CPU em contenção de thumbnails.

## Prioridade Baixa
6) Estado do player sem lock pesado
   - Trocar `Mutex<VideoState>` por `RwLock` ou atomics + snapshot leve.
   - Evita micro-stutter em leituras frequentes.

7) Revisão de cores hardcoded (somente após perf)
   - Centralizar em `ui::theme`; manter impacto zero em CPU.

## Métricas e Verificação
- Medir FPS e tempo de frame antes/depois (profiling egui repaint time).
- Abrir pasta grande (10k itens) e medir CPU média da UI thread.
- Abrir vídeo grande e medir tempo para primeiro frame + ausência de travas durante seek.
