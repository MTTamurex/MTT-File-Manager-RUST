# REVISÃO DE CÓDIGO HOLÍSTICA (FOCO PERFORMANCE + UI/UX)

**Data:** 15/01/2026  \
**Autor:** Copilot (GPT-5.1-Codex-Max)  \
**Escopo:** Perf (CPU/Mem) + Modernização UI/UX para egui + WebView2

## Achados Prioritários

1) BLOQUEIO NA MAIN THREAD DURANTE INIT DO WEBVIEW  
- **Onde:** [src/ui/components/webview_preview.rs](src/ui/components/webview_preview.rs#L230-L340) (\`init_webview\` é chamado dentro de \`update\`).  
- **Problema:** \`start_video_server\` + \`probe_video_codecs_internal\` (ffprobe) rodam síncronos na thread de UI. Em vídeos grandes ou em rede isso congela o frame (jank) e atrasa interações.  
- **Solução:** Mover probe+bind para worker rápido antes de construir o WebView. Exemplo: disparar thread que retorna `(port, codec_info)` via channel e, enquanto isso, renderizar spinner em egui; só construir o WebView quando o resultado chegar.  
```rust
// no WebviewPreview
struct PendingInit {
    rx: std::sync::mpsc::Receiver<(u16, VideoCodecInfo)>;
}

pub fn update(&mut self, ui: &mut egui::Ui, frame: Option<&eframe::Frame>) {
    if self.pending_init.is_none() {
        let (tx, rx) = mpsc::channel();
        let path = self.path.clone();
        thread::spawn(move || {
            if let Some(port) = start_video_server_bg(&path) {
                let info = probe_video_codecs_internal(&path);
                let _ = tx.send((port, info));
            }
        });
        self.pending_init = Some(PendingInit { rx });
        ui.add(egui::Spinner());
        return;
    }
    if let Some(init) = &self.pending_init {
        if let Ok((port, info)) = init.rx.try_recv() {
            self.finish_webview_init(ui, frame, port, info);
        } else {
            ui.add(egui::Spinner());
            return;
        }
    }
    // ... resto do update ...
}
```
- **Prioridade:** Alta (jank perceptível / risco de freeze em rede).

2) SERVIDOR DE VÍDEO SEM SHUTDOWN (LEAK DE THREAD/PORTA)  
- **Onde:** [src/ui/components/webview_preview.rs](src/ui/components/webview_preview.rs#L60-L190) (\`start_video_server\` spawna thread; _server_shutdown não é usado).  
- **Problema:** Cada instância cria listener + thread que nunca recebe sinal de término. Ao trocar de arquivo/aba, portas ficam vivas até sair do app → vazamento de threads/handles.  
- **Solução:** Implementar \`Drop\` para enviar shutdown e ocultar WebView.  
```rust
impl Drop for WebviewPreview {
    fn drop(&mut self) {
        if let Some(tx) = self._server_shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(wv) = self.webview.take() {
            let _ = wv.set_visible(false);
        }
    }
}
```
- **Prioridade:** Alta (recursos de SO vazando em uso prolongado).

3) REPINTA INCONDICIONAL E IPC A CADA 250ms  
- **Onde:** [src/ui/components/webview_preview.rs](src/ui/components/webview_preview.rs#L530-L620) (\`update\`) e JS embutido em \`init_webview\` (interval 250ms).  
- **Problema:** \`ui.ctx().request_repaint_after(250ms)\` roda mesmo com player oculto/pausado, e IPC JSON é parseado a cada tick. Em múltiplos vídeos abertos, gera repaints e parse JSON desnecessários.  
- **Solução:** Gate por visibilidade/estado e reduzir frequência quando o vídeo está parado.  
```rust
// Rust
let state = self.state.read().unwrap().clone();
let repaint_ms = if self.is_visible && state.is_playing { 120 } else { 600 };
ui.ctx().request_repaint_after(Duration::from_millis(repaint_ms));

// JS (dentro do HTML)
const TICK_MS = video.paused ? 600 : 200;
setInterval(() => { /* postMessage ... */ }, TICK_MS);
```
- **Prioridade:** Média (CPU evitável em idle/abas ocultas).

4) STORM DE REPINTA EM WORKERS DE THUMB/FOLDER PREVIEW  
- **Onde:** [src/workers/thumbnail_worker.rs](src/workers/thumbnail_worker.rs#L100-L220) e [src/workers/folder_preview_worker.rs](src/workers/folder_preview_worker.rs#L40-L90) chamam \`ctx.request_repaint()\` a cada item.  
- **Problema:** Ao carregar muitas thumbs/pastas, centenas de repaints por segundo são agendados, saturando a main thread de egui.  
- **Solução:** Fazer throttle de repaints (ex.: um repaint a cada 16–33ms) e/ou só disparar ao final de lote.  
```rust
fn throttled_repaint(ctx: &egui::Context, last: &AtomicU64) {
    let now = now_millis();
    let prev = last.swap(now, Ordering::Relaxed);
    if now.saturating_sub(prev) > 16 {
        ctx.request_repaint();
    }
}
// Usar em vez de request_repaint() direto nos workers.
```
- **Prioridade:** Média (picos de CPU na UI).

5) LOCK + CLONE DO ESTADO DE VÍDEO EM HOT PATH  
- **Onde:** [src/ui/components/webview_preview.rs](src/ui/components/webview_preview.rs#L55-L110) (\`get_state\` e controles).  
- **Problema:** \`self.state.lock().unwrap().clone()\` em cada leitura → contenção e alocação em loops de renderização.  
- **Solução:** Trocar para \`RwLock\` ou atomics para campos simples; expor snapshot leve.  
```rust
#[derive(Default, Clone, Copy)]
struct VideoStateSnap { is_playing: bool, current_time: f64, duration: f64, volume: f32, is_muted: bool }

pub fn get_state(&self) -> VideoStateSnap {
    let s = self.state.read().unwrap();
    VideoStateSnap { is_playing: s.is_playing, current_time: s.current_time, duration: s.duration, volume: s.volume, is_muted: s.is_muted }
}
```
- **Prioridade:** Baixa (micro-stutter em listas longas de render).

6) UI/UX: CORES HARD-CODED E VISUAL PLANO NA TOOLBAR/TABS  
- **Onde:** [src/ui/app_impl.rs](src/ui/app_impl.rs#L42-L110) (cores RGB fixas em \`render_tab_bar_layer\` e \`render_toolbar_layer\`).  
- **Problema:** Visual utilitário e inconsistente com tema global; sem sombras/bordas para hierarquia visual.  
- **Solução:** Centralizar cores em \`ui::theme\`, aplicar bordas/arredondamento e leve sombra para aparência “sleek” sem custo de perf.  
```rust
let theme = crate::ui::theme::current(ctx);
egui::TopBottomPanel::top("tab_bar_panel")
    .frame(egui::Frame::none()
        .fill(theme.surface)
        .rounding(egui::Rounding::same(6.0))
        .shadow(egui::epaint::Shadow::small_dark()))
    .show(ctx, |ui| { /* ... */ });
```
- **Prioridade:** Baixa (estético / consistência).

## Recomendações Resumidas
- Priorize mover probe ffmpeg + bind do servidor para background antes de criar WebView (elimina travas visíveis). 
- Adicione \`Drop\` no player para encerrar servidor e ocultar WebView ao trocar de arquivo/aba. 
- Throttle repaints (WebView + workers) e reduza a cadência de IPC quando o player estiver parado/oculto. 
- Simplifique estado do player para leituras lock-free (RwLock/atomics). 
- Aplicar tema centralizado nas barras superiores para um visual mais profissional.

## Comparação com CODE_REVIEW_REPORT.md
- **Novo foco:** Bloqueio no init do WebView (ffprobe + servidor) e leak de servidor não estavam detalhados no relatório anterior. 
- **Repaint storm:** O relatório antigo menciona IPC/JSON, mas não o excesso de repaints nos workers; agora endereçado com throttle. 
- **UI:** Antes sugeria temas genéricos; agora há ponto específico de hardcode na toolbar/tab bar com snippet pronto. 
- **Itens mantidos:** Contenção por locks e overhead de IPC continuam relevantes; as soluções aqui refinam a mitigação (throttle + snapshots).
