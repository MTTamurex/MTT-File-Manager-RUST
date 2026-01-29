# Plano de Implementacao - Otimizacoes HDD Mecanico (Fase 2)

**Data:** 2026-01-29
**Projeto:** MTT File Manager
**Pre-requisitos:** Implementacoes do `PLANO_OTIMIZACAO_HDD_AVANCADO.md` ja realizadas

---

## Resumo das Otimizacoes

| Fase | Otimizacao | Impacto | Esforco | Risco |
|------|------------|---------|---------|-------|
| 1 | Thumbnail Prefetch por Viewport | Alto | Baixo | Baixo |
| 2 | Prefetch Preditivo de Diretorios | Alto | Medio | Baixo |
| 3 | Cache Warm-up no Idle | Medio | Medio | Baixo |
| 4 | Batch Sizing Adaptativo | Medio | Baixo | Baixo |
| 5 | Read Coalescing | Medio | Alto | Medio |
| 6 | Memory-Mapped Directory Index | Medio | Alto | Alto |

> **⚠️ IMPORTANTE:** A Fase 6 (Memory-Mapped Directory Index) NAO deve ser implementada automaticamente.
> Aguardar solicitacao explicita do usuario antes de iniciar esta fase.
> Motivos: alta complexidade, requer mudanca arquitetural significativa, trade-offs de flexibilidade.

---

## Fase 1: Thumbnail Prefetch por Viewport

### 1.1 Descricao

Pre-carregar thumbnails de itens que estao proximos ao viewport visivel, antes do usuario fazer scroll. Isso elimina o delay perceptivel ao navegar pela lista.

### 1.2 Conceito

```
┌─────────────────────────┐
│   [Prefetch Zone -N]    │  ← Rows acima do viewport (scroll up)
├─────────────────────────┤
│                         │
│    Viewport Visivel     │  ← Prioridade Interactive
│    (items renderizados) │
│                         │
├─────────────────────────┤
│   [Prefetch Zone +N]    │  ← Rows abaixo do viewport (scroll down)
└─────────────────────────┘

N = numero de rows a pre-carregar (sugestao: 2-3 rows)
```

### 1.3 Impacto

- **Beneficio:** Thumbnails ja carregados quando entram no viewport (~90% hit rate)
- **Risco:** Nenhum (usa sistema de prioridades existente)
- **Overhead:** Minimo (apenas muda prioridade de requisicoes)

### 1.4 Arquivos a Modificar

#### 1.4.1 `src/ui/grid.rs` (ou equivalente de renderizacao)

Identificar o range de indices visiveis e calcular zona de prefetch:

```rust
// Estrutura para tracking de viewport
pub struct ViewportTracker {
    pub first_visible_index: usize,
    pub last_visible_index: usize,
    pub prefetch_rows: usize,  // Quantas rows pre-carregar
    pub columns: usize,
}

impl ViewportTracker {
    /// Calcula indices para prefetch baseado no viewport atual
    pub fn get_prefetch_range(&self, total_items: usize) -> (usize, usize) {
        let items_per_prefetch = self.prefetch_rows * self.columns;

        // Prefetch acima do viewport
        let prefetch_start = self.first_visible_index
            .saturating_sub(items_per_prefetch);

        // Prefetch abaixo do viewport
        let prefetch_end = (self.last_visible_index + items_per_prefetch)
            .min(total_items);

        (prefetch_start, prefetch_end)
    }

    /// Retorna indices que devem ter prioridade Prefetch (nao Interactive)
    pub fn get_prefetch_indices(&self, total_items: usize) -> Vec<usize> {
        let (start, end) = self.get_prefetch_range(total_items);

        (start..end)
            .filter(|&i| i < self.first_visible_index || i > self.last_visible_index)
            .collect()
    }
}
```

#### 1.4.2 Integracao com `thumbnail_queue.push()`

Modificar a logica de requisicao de thumbnails para usar prioridade correta:

```rust
// ANTES: Todas requisicoes com mesma prioridade
for index in visible_range {
    thumbnail_queue.push(path, gen, size, IOPriority::Interactive);
}

// DEPOIS: Prioridade baseada em visibilidade
let viewport = ViewportTracker {
    first_visible_index,
    last_visible_index,
    prefetch_rows: if is_ssd { 1 } else { 3 }, // HDD precisa mais prefetch
    columns: grid_columns,
};

let (prefetch_start, prefetch_end) = viewport.get_prefetch_range(total_items);

for index in prefetch_start..prefetch_end {
    let priority = if index >= first_visible_index && index <= last_visible_index {
        IOPriority::Interactive  // Visivel agora
    } else {
        IOPriority::Prefetch     // Provavelmente visivel em breve
    };

    thumbnail_queue.push(items[index].path.clone(), gen, size, priority);
}
```

#### 1.4.3 Ajuste de `prefetch_rows` baseado em velocidade de scroll

Opcional: aumentar prefetch zone quando usuario esta fazendo scroll rapido:

```rust
pub struct ScrollTracker {
    last_scroll_position: f32,
    last_scroll_time: Instant,
    scroll_velocity: f32,  // pixels/segundo
}

impl ScrollTracker {
    pub fn update(&mut self, current_position: f32) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_scroll_time).as_secs_f32();

        if dt > 0.0 {
            self.scroll_velocity = (current_position - self.last_scroll_position).abs() / dt;
        }

        self.last_scroll_position = current_position;
        self.last_scroll_time = now;
    }

    /// Retorna rows extras para prefetch baseado na velocidade
    pub fn extra_prefetch_rows(&self) -> usize {
        match self.scroll_velocity {
            v if v < 500.0 => 0,   // Scroll lento
            v if v < 1500.0 => 2,  // Scroll medio
            _ => 4,                 // Scroll rapido
        }
    }
}
```

### 1.5 Testes

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewport_prefetch_range() {
        let tracker = ViewportTracker {
            first_visible_index: 10,
            last_visible_index: 25,
            prefetch_rows: 2,
            columns: 5,
        };

        let (start, end) = tracker.get_prefetch_range(100);

        // Prefetch: 2 rows * 5 cols = 10 items antes e depois
        assert_eq!(start, 0);   // 10 - 10 = 0 (clamped)
        assert_eq!(end, 35);    // 25 + 10 = 35
    }

    #[test]
    fn test_prefetch_indices_excludes_visible() {
        let tracker = ViewportTracker {
            first_visible_index: 10,
            last_visible_index: 15,
            prefetch_rows: 1,
            columns: 5,
        };

        let indices = tracker.get_prefetch_indices(100);

        // Nao deve incluir indices visiveis (10-15)
        assert!(indices.iter().all(|&i| i < 10 || i > 15));
    }
}
```

---

## Fase 2: Prefetch Preditivo de Diretorios

### 2.1 Descricao

Prever quais diretorios o usuario provavelmente vai acessar e pre-carregar seus listings em background.

### 2.2 Heuristicas de Predicao

| Heuristica | Prioridade | Razao |
|------------|------------|-------|
| Pasta pai | Alta | Usuario frequentemente volta |
| Irmas da pasta atual | Media | Navegacao lateral comum |
| Primeira subpasta visivel | Media | Provavel proximo destino |
| Historico recente (ultimas 5) | Media | Padroes de uso |
| Pastas favoritas/fixadas | Baixa | Acesso eventual |

### 2.3 Impacto

- **Beneficio:** ~30-50% menos tempo de espera ao navegar
- **Risco:** Baixo (usa IOPriority::Background)
- **Overhead:** Uso de CPU/IO em background, limitado por rate limiting

### 2.4 Arquivos a Modificar

#### 2.4.1 Criar `src/workers/predictive_prefetch.rs`

```rust
//! Predictive directory prefetching based on navigation patterns
//!
//! Predicts which directories the user is likely to navigate to and
//! pre-loads their listings in background to eliminate wait times.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::directory_index::DirectoryIndex;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::infrastructure::ntfs_reader;

/// Maximum directories to prefetch per prediction cycle
const MAX_PREFETCH_PER_CYCLE: usize = 5;

/// Minimum interval between prefetch cycles (avoid disk thrashing)
const MIN_PREFETCH_INTERVAL: Duration = Duration::from_millis(500);

/// Messages for the predictive prefetch worker
pub enum PredictiveMessage {
    /// User navigated to a new directory
    NavigatedTo(PathBuf),
    /// User's navigation history updated
    HistoryUpdated(Vec<PathBuf>),
    /// Shutdown the worker
    Shutdown,
}

/// Predictions for what to prefetch
#[derive(Debug)]
struct PrefetchPrediction {
    path: PathBuf,
    confidence: f32,  // 0.0 - 1.0
    reason: &'static str,
}

pub struct PredictivePrefetcher {
    current_path: Option<PathBuf>,
    history: VecDeque<PathBuf>,
    last_prefetch: Instant,
}

impl PredictivePrefetcher {
    pub fn new() -> Self {
        Self {
            current_path: None,
            history: VecDeque::with_capacity(10),
            last_prefetch: Instant::now(),
        }
    }

    /// Generate predictions based on current state
    pub fn predict(&self) -> Vec<PrefetchPrediction> {
        let mut predictions = Vec::new();

        let Some(current) = &self.current_path else {
            return predictions;
        };

        // 1. Parent directory (high confidence - user often goes back)
        if let Some(parent) = current.parent() {
            predictions.push(PrefetchPrediction {
                path: parent.to_path_buf(),
                confidence: 0.9,
                reason: "parent_directory",
            });
        }

        // 2. Sibling directories (medium confidence)
        if let Some(parent) = current.parent() {
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.filter_map(|e| e.ok()).take(5) {
                    let path = entry.path();
                    if path.is_dir() && path != *current {
                        predictions.push(PrefetchPrediction {
                            path,
                            confidence: 0.5,
                            reason: "sibling_directory",
                        });
                    }
                }
            }
        }

        // 3. First subdirectories (medium confidence)
        if let Ok(entries) = std::fs::read_dir(current) {
            for entry in entries.filter_map(|e| e.ok()).take(3) {
                let path = entry.path();
                if path.is_dir() {
                    predictions.push(PrefetchPrediction {
                        path,
                        confidence: 0.6,
                        reason: "first_subdirectory",
                    });
                }
            }
        }

        // 4. Recent history (medium confidence)
        for (i, hist_path) in self.history.iter().enumerate() {
            if hist_path != current {
                predictions.push(PrefetchPrediction {
                    path: hist_path.clone(),
                    confidence: 0.4 - (i as f32 * 0.05), // Decay with age
                    reason: "recent_history",
                });
            }
        }

        // Sort by confidence (highest first) and deduplicate
        predictions.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        predictions.dedup_by(|a, b| a.path == b.path);
        predictions.truncate(MAX_PREFETCH_PER_CYCLE);

        predictions
    }

    /// Update state when user navigates
    pub fn on_navigate(&mut self, path: PathBuf) {
        // Add to history
        if self.history.front() != Some(&path) {
            self.history.push_front(path.clone());
            if self.history.len() > 10 {
                self.history.pop_back();
            }
        }

        self.current_path = Some(path);
    }
}

/// Spawn the predictive prefetch worker
pub fn spawn_predictive_prefetcher(
    receiver: Receiver<PredictiveMessage>,
    directory_cache: Arc<DirectoryCache>,
    directory_index: Option<Arc<DirectoryIndex>>,
) {
    std::thread::spawn(move || {
        // Set background priority for this thread
        io_priority::set_thread_priority(IOPriority::Background);

        let mut prefetcher = PredictivePrefetcher::new();

        loop {
            // Non-blocking check for messages
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(PredictiveMessage::NavigatedTo(path)) => {
                    prefetcher.on_navigate(path);
                }
                Ok(PredictiveMessage::HistoryUpdated(history)) => {
                    prefetcher.history = history.into_iter().collect();
                }
                Ok(PredictiveMessage::Shutdown) => {
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Continue to prefetch logic
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }

            // Rate limit prefetch operations
            if prefetcher.last_prefetch.elapsed() < MIN_PREFETCH_INTERVAL {
                continue;
            }

            // Generate and execute predictions
            let predictions = prefetcher.predict();

            for prediction in predictions {
                // Skip if already cached
                if directory_cache.get(&prediction.path).is_some() {
                    continue;
                }

                // Skip if indexed and not changed
                if let Some(ref index) = directory_index {
                    if !index.might_have_changed(&prediction.path) {
                        continue;
                    }
                }

                // Prefetch using fast NTFS reader
                if let Some(entries) = ntfs_reader::read_directory_fast(&prediction.path) {
                    // Convert to FileEntry and cache
                    let file_entries: Vec<crate::domain::file_entry::FileEntry> = entries
                        .into_iter()
                        .filter(|e| {
                            let is_hidden = (e.attributes & 0x02) != 0;
                            let is_system = (e.attributes & 0x04) != 0;
                            !is_hidden && !is_system && !e.name.starts_with('.')
                        })
                        .map(|e| crate::domain::file_entry::FileEntry {
                            path: prediction.path.join(&e.name),
                            name: e.name,
                            is_dir: e.is_dir,
                            size: if e.is_dir { 0 } else { e.size },
                            modified: e.modified,
                            folder_cover: None,
                            drive_info: None,
                            sync_status: crate::domain::file_entry::SyncStatus::None,
                            deletion_date: None,
                            recycle_original_path: None,
                        })
                        .collect();

                    directory_cache.put(prediction.path.clone(), file_entries);

                    eprintln!(
                        "[Prefetch] Predicted and cached: {:?} (reason: {})",
                        prediction.path.file_name(),
                        prediction.reason
                    );
                }
            }

            prefetcher.last_prefetch = Instant::now();
        }
    });
}
```

#### 2.4.2 Registrar modulo em `src/workers/mod.rs`

```rust
pub mod predictive_prefetch;
```

#### 2.4.3 Integrar com `ImageViewerApp`

Adicionar em `src/app/state.rs`:

```rust
pub predictive_sender: Sender<PredictiveMessage>,
```

Inicializar em `src/app/init.rs`:

```rust
let (predictive_sender, predictive_receiver) = std::sync::mpsc::channel();

// Spawn worker
crate::workers::predictive_prefetch::spawn_predictive_prefetcher(
    predictive_receiver,
    directory_cache.clone(),
    directory_index.clone(),
);
```

Notificar navegacao em `load_folder()`:

```rust
// No inicio de load_folder, apos definir current_path
let _ = self.predictive_sender.send(
    PredictiveMessage::NavigatedTo(PathBuf::from(&self.current_path))
);
```

---

## Fase 3: Cache Warm-up no Idle

### 3.1 Descricao

Durante periodos de inatividade do usuario (sem input por N segundos), executar operacoes de pre-carregamento em background ultra-baixa prioridade.

### 3.2 Impacto

- **Beneficio:** Thumbnails e listings prontos quando usuario voltar
- **Risco:** Baixo (cancela imediatamente quando ha input)
- **Overhead:** Apenas durante idle, prioridade muito baixa

### 3.3 Arquivos a Modificar

#### 3.3.1 Criar `src/workers/idle_warmup.rs`

```rust
//! Idle-time cache warming
//!
//! Performs background cache warming when the user is idle.
//! Immediately cancels when user activity is detected.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::infrastructure::directory_cache::DirectoryCache;
use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::io_priority::{self, IOPriority};
use crate::workers::thumbnail_worker::PriorityThumbnailQueue;

/// Time without user input before starting idle warm-up
const IDLE_THRESHOLD: Duration = Duration::from_secs(5);

/// Time between warm-up operations (avoid disk saturation)
const WARMUP_INTERVAL: Duration = Duration::from_millis(200);

pub enum IdleWarmupMessage {
    /// User activity detected - pause warming
    UserActive,
    /// Current directory changed
    CurrentDirectory(PathBuf),
    /// Visible items that need thumbnails
    VisibleItems(Vec<PathBuf>),
    /// Shutdown
    Shutdown,
}

pub struct IdleWarmupWorker {
    last_activity: Instant,
    current_directory: Option<PathBuf>,
    pending_thumbnails: Vec<PathBuf>,
    is_warming: bool,
}

impl IdleWarmupWorker {
    pub fn new() -> Self {
        Self {
            last_activity: Instant::now(),
            current_directory: None,
            pending_thumbnails: Vec::new(),
            is_warming: false,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.last_activity.elapsed() >= IDLE_THRESHOLD
    }

    pub fn on_activity(&mut self) {
        self.last_activity = Instant::now();
        if self.is_warming {
            eprintln!("[IdleWarmup] User activity detected, pausing warm-up");
            self.is_warming = false;
        }
    }
}

pub fn spawn_idle_warmup_worker(
    receiver: Receiver<IdleWarmupMessage>,
    thumbnail_queue: Arc<PriorityThumbnailQueue>,
    directory_cache: Arc<DirectoryCache>,
    current_generation: Arc<std::sync::atomic::AtomicUsize>,
) {
    std::thread::spawn(move || {
        // Ultra-low priority
        io_priority::set_thread_priority(IOPriority::Background);

        let mut worker = IdleWarmupWorker::new();
        let mut last_warmup = Instant::now();

        loop {
            // Check for messages (non-blocking)
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(IdleWarmupMessage::UserActive) => {
                    worker.on_activity();
                }
                Ok(IdleWarmupMessage::CurrentDirectory(path)) => {
                    worker.current_directory = Some(path);
                }
                Ok(IdleWarmupMessage::VisibleItems(items)) => {
                    worker.pending_thumbnails = items;
                }
                Ok(IdleWarmupMessage::Shutdown) => {
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }

            // Only warm up during idle
            if !worker.is_idle() {
                continue;
            }

            // Rate limit warm-up operations
            if last_warmup.elapsed() < WARMUP_INTERVAL {
                continue;
            }

            if !worker.is_warming {
                eprintln!("[IdleWarmup] Starting idle warm-up cycle");
                worker.is_warming = true;
            }

            // Warm up thumbnails for visible items that aren't loaded yet
            if let Some(path) = worker.pending_thumbnails.pop() {
                let gen = current_generation.load(Ordering::Relaxed);
                thumbnail_queue.push(path, gen, 256, IOPriority::Background);
                last_warmup = Instant::now();
            }

            // If no pending thumbnails, try to warm up subdirectories
            if worker.pending_thumbnails.is_empty() {
                if let Some(ref current) = worker.current_directory {
                    if let Some(entries) = directory_cache.get(current) {
                        // Find subdirectories not yet cached
                        for entry in entries.iter().filter(|e| e.is_dir).take(3) {
                            if directory_cache.get(&entry.path).is_none() {
                                // Trigger prefetch for subdirectory
                                // (Uses existing prefetch mechanism)
                            }
                        }
                    }
                }
            }
        }
    });
}
```

#### 3.3.2 Detectar User Activity

Modificar o loop principal da UI para detectar input:

```rust
// Em algum ponto do update loop da UI
if ctx.input(|i| i.pointer.any_click() || i.keys_down.len() > 0 || i.scroll_delta != Vec2::ZERO) {
    let _ = idle_warmup_sender.send(IdleWarmupMessage::UserActive);
}
```

---

## Fase 4: Batch Sizing Adaptativo

### 4.1 Descricao

Ajustar dinamicamente o tamanho dos batches de carregamento baseado no tipo de disco, quantidade de arquivos na pasta, e velocidade observada.

### 4.2 Impacto

- **Beneficio:** Melhor responsividade em pastas pequenas, melhor throughput em pastas grandes
- **Risco:** Nenhum
- **Overhead:** Nenhum (apenas calculo)

### 4.3 Arquivos a Modificar

#### 4.3.1 Criar `src/infrastructure/adaptive_batch.rs`

```rust
//! Adaptive batch sizing for directory loading
//!
//! Adjusts batch sizes based on disk type, folder size, and observed performance.

use std::time::{Duration, Instant};

/// Minimum batch size (for responsiveness)
const MIN_BATCH_SIZE: usize = 25;

/// Maximum batch size (for memory and UI limits)
const MAX_BATCH_SIZE: usize = 1000;

/// Target time per batch in milliseconds
const TARGET_BATCH_MS: u64 = 50;

pub struct AdaptiveBatchConfig {
    pub is_ssd: bool,
    pub total_items: Option<usize>,
}

impl AdaptiveBatchConfig {
    /// Calculate initial batch size before loading starts
    pub fn initial_batch_size(&self) -> usize {
        match (self.is_ssd, self.total_items) {
            // SSD: Large batches for throughput
            (true, _) => 500,

            // HDD + small folder: Load all at once
            (false, Some(n)) if n <= 50 => n.max(MIN_BATCH_SIZE),

            // HDD + medium folder: Smaller batches for responsiveness
            (false, Some(n)) if n <= 200 => 50,

            // HDD + large folder: Standard batch size
            (false, Some(n)) if n <= 1000 => 100,

            // HDD + very large folder: Slightly larger batches
            (false, Some(_)) => 150,

            // HDD + unknown size: Conservative default
            (false, None) => 75,
        }
    }
}

/// Tracks batch performance and adjusts dynamically
pub struct AdaptiveBatchTracker {
    is_ssd: bool,
    batch_times: Vec<Duration>,
    current_batch_size: usize,
}

impl AdaptiveBatchTracker {
    pub fn new(config: AdaptiveBatchConfig) -> Self {
        Self {
            is_ssd: config.is_ssd,
            batch_times: Vec::with_capacity(10),
            current_batch_size: config.initial_batch_size(),
        }
    }

    /// Record a batch completion and adjust size if needed
    pub fn record_batch(&mut self, duration: Duration, items_processed: usize) {
        self.batch_times.push(duration);

        // Keep last 5 measurements
        if self.batch_times.len() > 5 {
            self.batch_times.remove(0);
        }

        // SSDs don't need adaptation
        if self.is_ssd {
            return;
        }

        // Calculate average time per item
        let avg_time_per_item = self.batch_times.iter()
            .map(|d| d.as_micros() as f64)
            .sum::<f64>() / (items_processed as f64 * self.batch_times.len() as f64);

        // Target: ~50ms per batch
        let target_items = (TARGET_BATCH_MS as f64 * 1000.0 / avg_time_per_item) as usize;

        // Smooth adjustment (don't change too drastically)
        let new_size = (self.current_batch_size + target_items) / 2;
        self.current_batch_size = new_size.clamp(MIN_BATCH_SIZE, MAX_BATCH_SIZE);
    }

    pub fn batch_size(&self) -> usize {
        self.current_batch_size
    }
}
```

#### 4.3.2 Integrar em `folder_loading.rs`

```rust
// No inicio do thread de scan
use crate::infrastructure::adaptive_batch::{AdaptiveBatchConfig, AdaptiveBatchTracker};

let config = AdaptiveBatchConfig {
    is_ssd,
    total_items: directory_index
        .as_ref()
        .and_then(|di| di.get_directory(&PathBuf::from(&base_path)))
        .map(|(meta, _)| meta.file_count),
};

let mut batch_tracker = AdaptiveBatchTracker::new(config);

// No loop de processamento
let batch_start = Instant::now();
// ... processar batch ...
batch_tracker.record_batch(batch_start.elapsed(), batch.len());

// Para proximo batch
let batch_size = batch_tracker.batch_size();
```

---

## Fase 5: Read Coalescing

### 5.1 Descricao

Ordenar requisicoes de leitura para processar arquivos na ordem em que aparecem no diretorio, minimizando seeks do cabecote do HDD.

### 5.2 Impacto

- **Beneficio:** ~20-30% menos seeks em operacoes de thumbnail
- **Risco:** Medio (requer modificacao na fila de prioridades)
- **Complexidade:** Alta

### 5.3 Conceito

```
Requisicoes originais (ordem de chegada):
  Item_05, Item_12, Item_03, Item_08, Item_01

Apos ordenacao por posicao no diretorio:
  Item_01, Item_03, Item_05, Item_08, Item_12
```

### 5.4 Arquivos a Modificar

#### 5.4.1 Adicionar indice de posicao em `ThumbnailRequest`

Em `src/workers/thumbnail_worker.rs`:

```rust
#[derive(Debug, Clone)]
struct ThumbnailRequest {
    path: PathBuf,
    generation: usize,
    size: u32,
    priority: IOPriority,
    directory_index: Option<usize>,  // NOVO: posicao no diretorio
}
```

#### 5.4.2 Modificar `push()` para aceitar indice

```rust
impl PriorityThumbnailQueue {
    pub fn push_with_index(
        &self,
        path: PathBuf,
        gen: usize,
        request_size: u32,
        priority: IOPriority,
        directory_index: Option<usize>,
    ) {
        // ... existing deduplication logic ...

        let request = ThumbnailRequest {
            path,
            generation: gen,
            size: request_size,
            priority,
            directory_index,
        };

        state.by_directory.entry(parent).or_default().push(request);

        // NOVO: Re-ordenar por indice de diretorio (apenas para HDD)
        if !state.is_ssd.unwrap_or(true) {
            if let Some(items) = state.by_directory.get_mut(&parent) {
                items.sort_by(|a, b| {
                    // Primeiro por prioridade, depois por indice de diretorio
                    match a.priority.cmp(&b.priority) {
                        std::cmp::Ordering::Equal => {
                            a.directory_index.cmp(&b.directory_index)
                        }
                        other => other,
                    }
                });
            }
        }

        self.condvar.notify_one();
    }
}
```

#### 5.4.3 Modificar chamadas para passar indice

No codigo que requisita thumbnails:

```rust
// Quando iterando sobre items com indice conhecido
for (index, item) in items.iter().enumerate() {
    thumbnail_queue.push_with_index(
        item.path.clone(),
        gen,
        size,
        priority,
        Some(index),  // Passa o indice
    );
}
```

---

## Fase 6: Memory-Mapped Directory Index (OPCIONAL)

> **⛔ NAO IMPLEMENTAR SEM ORDEM EXPLICITA DO USUARIO**
>
> Esta fase so deve ser iniciada quando o usuario solicitar explicitamente.
> Nao faz parte do fluxo padrao de otimizacoes.

### 6.1 Descricao

Substituir ou complementar o SQLite com um arquivo memory-mapped para diretorios frequentemente acessados, eliminando overhead de parsing SQL.

### 6.2 Quando Usar

**Casos de uso adequados:**
- Diretorios muito grandes (>10.000 arquivos)
- Navegacao muito frequente entre poucas pastas
- Quando profiling mostrar SQLite como gargalo

**NAO adequado para:**
- Uso geral (SQLite e suficiente)
- Quando flexibilidade de queries e importante

### 6.3 Requisitos

| Requisito | Motivo |
|-----------|--------|
| Profiling mostrando SQLite como gargalo | Justificar complexidade |
| Formato binario bem definido | Compatibilidade entre versoes |
| Estrategia de migracao | Nao perder dados existentes |

### 6.4 Impacto

- **Beneficio:** ~50% menos latencia para leitura de diretorios hot
- **Risco:** Alto (formato binario, complexidade de manutencao)
- **Complexidade:** Muito alta

### 6.5 Estrutura Proposta (Apenas Referencia)

```rust
// Formato binario do arquivo mmap
#[repr(C)]
struct MmapHeader {
    magic: [u8; 4],      // "MTTD"
    version: u32,
    entry_count: u32,
    data_offset: u32,
}

#[repr(C)]
struct MmapEntry {
    name_offset: u32,    // Offset para string pool
    name_len: u16,
    flags: u16,          // is_dir, is_hidden, etc.
    size: u64,
    modified: u64,
}

// Estrutura em memoria
struct MmapDirectoryIndex {
    mmap: memmap2::Mmap,
    header: &MmapHeader,
    entries: &[MmapEntry],
    string_pool: &[u8],
}
```

### 6.6 Trade-offs

| SQLite (Atual) | Memory-Mapped |
|---------------|---------------|
| ✅ Queries flexiveis | ❌ Formato fixo |
| ✅ ACID transactions | ❌ Corrupcao possivel |
| ✅ Ferramentas de debug | ❌ Binario opaco |
| ❌ Overhead de parsing | ✅ Acesso direto |
| ❌ Latencia de I/O | ✅ Paginas em memoria |

---

## Ordem de Implementacao Recomendada

```
Fase 1: Thumbnail Prefetch por Viewport    [Sem dependencias, baixo risco, alto impacto]
Fase 2: Prefetch Preditivo                 [Sem dependencias, baixo risco, alto impacto]
Fase 4: Batch Sizing Adaptativo            [Sem dependencias, baixo risco, medio impacto]
Fase 3: Cache Warm-up no Idle              [Depende de deteccao de idle]
Fase 5: Read Coalescing                    [Modificacao de estruturas existentes]
Fase 6: Memory-Mapped Index                [OPCIONAL - NAO IMPLEMENTAR SEM ORDEM EXPLICITA]
```

---

## Metricas de Sucesso

| Metrica | Baseline | Target |
|---------|----------|--------|
| Tempo para primeiro thumbnail visivel | ~500ms | <200ms |
| Tempo de navegacao entre pastas | ~800ms | <300ms |
| Thumbnails prontos ao scrollar | ~60% | >90% |
| Uso de CPU em idle | 0% | <2% |

---

## Notas de Implementacao

1. **Logging:** Adicionar logs de performance com prefixo `[PERF]` para facilitar profiling
2. **Feature Flags:** Considerar usar feature flags para habilitar/desabilitar otimizacoes individualmente
3. **Fallbacks:** Sempre ter fallback para comportamento original em caso de erro
4. **Testes:** Cada fase deve incluir testes unitarios e de integracao
5. **Documentacao:** Atualizar README com novas otimizacoes implementadas
