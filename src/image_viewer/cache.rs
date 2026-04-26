use crate::image_viewer::loader;
use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadPriority {
    Urgent,
    High,
    Normal,
}

#[derive(Debug)]
pub struct LoadOutput {
    pub index: usize,
    pub frame: io::Result<loader::DecodedFrame>,
}

#[derive(Clone, Debug)]
struct LoadJob {
    index: usize,
    path: PathBuf,
    priority: LoadPriority,
}

/// Cached GPU texture plus original resolution metadata.
struct CachedTexture {
    texture: egui::TextureHandle,
    original_width: u32,
    original_height: u32,
}

/// Sliding-window cache that stores decoded images as **GPU textures**
/// (`TextureHandle`) instead of CPU-side RGBA buffers.
///
/// This is the key architectural difference from the previous implementation
/// (and the reason viewskater-egui uses ~50-100 MB while we used ~200 MB+).
/// CPU `Vec<u8>` buffers are always resident in process working-set; GPU
/// textures live in DX12 heaps that Windows can manage separately — on a
/// dGPU they occupy VRAM, and even on iGPU (UMA) they're in committed
/// memory that the OS can page out of the working set.
pub struct WindowCache {
    radius: usize,
    items: HashMap<usize, CachedTexture>,
}

impl WindowCache {
    pub fn new(radius: usize) -> Self {
        Self {
            radius,
            items: HashMap::new(),
        }
    }

    pub fn radius(&self) -> usize {
        self.radius
    }

    pub fn has(&self, index: usize) -> bool {
        self.items.contains_key(&index)
    }

    /// Store a decoded image as a GPU texture in the cache.
    pub fn put(
        &mut self,
        index: usize,
        texture: egui::TextureHandle,
        original_width: u32,
        original_height: u32,
    ) {
        self.items.insert(
            index,
            CachedTexture {
                texture,
                original_width,
                original_height,
            },
        );
    }

    /// Clone the TextureHandle (cheap Arc increment) and return it with the
    /// original resolution. The entry stays in the cache.
    pub fn get(&self, index: usize) -> Option<(egui::TextureHandle, u32, u32)> {
        self.items.get(&index).map(|c| {
            (c.texture.clone(), c.original_width, c.original_height)
        })
    }

    /// Evict all entries outside the `[center - radius, center + radius]`
    /// window. Dropped `TextureHandle`s release their GPU memory.
    pub fn retain_window(&mut self, center: usize, total_len: usize) {
        if total_len == 0 {
            self.items.clear();
            return;
        }

        let min_idx = center.saturating_sub(self.radius);
        let max_idx = (center + self.radius).min(total_len.saturating_sub(1));
        self.items
            .retain(|&idx, _| idx >= min_idx && idx <= max_idx);
    }
}

pub struct PrefetchEngine {
    jobs_tx: Option<Sender<LoadJob>>,
    bg_jobs_tx: Option<Sender<LoadJob>>,
    urgent_job: Arc<std::sync::Mutex<Option<LoadJob>>>,
    urgent_notify_tx: Option<Sender<()>>,
    results_rx: Receiver<LoadOutput>,
    active_center: Arc<AtomicUsize>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    /// Worker thread handles retained until Drop, then detached without joining.
    /// Channel shutdown lets workers exit and run their COM/WIC cleanup.
    worker_handles: Vec<std::thread::JoinHandle<()>>,
}

impl PrefetchEngine {
    pub fn new(worker_count: usize, skip_radius: usize) -> Self {
        let worker_count = worker_count.clamp(1, 6);
        // Bounded channels sized to the cache window (2*radius+1).
        // This prevents unbounded queue growth during fast navigation.
        let window_size = skip_radius * 2 + 1;
        let (jobs_tx, jobs_rx) = crossbeam_channel::bounded::<LoadJob>(window_size);
        let (bg_jobs_tx, bg_jobs_rx) = crossbeam_channel::bounded::<LoadJob>(window_size);
        let (results_tx, results_rx) = crossbeam_channel::bounded::<LoadOutput>(window_size * 4);
        let urgent_job = Arc::new(std::sync::Mutex::new(None));
        let (urgent_notify_tx, urgent_notify_rx) = crossbeam_channel::bounded::<()>(1);
        let active_center = Arc::new(AtomicUsize::new(0));
        let repaint_ctx = Arc::new(OnceLock::<egui::Context>::new());
        let mut worker_handles = Vec::with_capacity(worker_count);

        for worker_id in 0..worker_count {
            let jobs_rx = jobs_rx.clone();
            let bg_jobs_rx = bg_jobs_rx.clone();
            let urgent_job = Arc::clone(&urgent_job);
            let urgent_notify_rx = urgent_notify_rx.clone();
            let results_tx = results_tx.clone();
            let active_center = Arc::clone(&active_center);
            let repaint_ctx = Arc::clone(&repaint_ctx);
            let spawn_result = std::thread::Builder::new()
                .name(format!("image-viewer-loader-{}", worker_id))
                .spawn(move || {
                    loop {
                        // Phase 1: Check urgent job (brief lock, released immediately).
                        // CRITICAL: The mutex must NOT be held during the blocking
                        // select! below — otherwise the UI thread deadlocks when
                        // trying to post a new urgent job while a worker blocks
                        // on empty channels.
                        let urgent = urgent_job
                            .lock()
                            .ok()
                            .and_then(|mut slot| slot.take());

                        let job = if let Some(job) = urgent {
                            Some(job)
                        } else {
                            // Phase 2: Try non-blocking read from high-priority channel.
                            match jobs_rx.try_recv() {
                                Ok(job) => Some(job),
                                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                                Err(crossbeam_channel::TryRecvError::Empty) => {
                                    // Phase 3: Block on channels (mutex NOT held!).
                                    // Includes urgent_notify so workers wake up
                                    // immediately when a new urgent job is posted.
                                    crossbeam_channel::select! {
                                        recv(urgent_notify_rx) -> _ => {
                                            // Urgent wake signal — re-check at top of loop.
                                            None
                                        },
                                        recv(jobs_rx) -> recv_res => match recv_res {
                                            Ok(job) => Some(job),
                                            Err(_) => break,
                                        },
                                        recv(bg_jobs_rx) -> recv_res => match recv_res {
                                            Ok(job) => Some(job),
                                            Err(_) => break,
                                        }
                                    }
                                }
                            }
                        };

                        let Some(job) = job else {
                            // Woke from urgent_notify — loop back to check mutex.
                            continue;
                        };

                        // Skip jobs for images too far from the current view.
                        let center = active_center.load(Ordering::Relaxed);
                        if job.index.abs_diff(center) > skip_radius + 2 {
                            // Notify the UI that this job was skipped so it can
                            // clear the entry from requested_jobs and retry later.
                            let _ = results_tx.try_send(LoadOutput {
                                index: job.index,
                                frame: Err(io::Error::new(
                                    io::ErrorKind::Interrupted,
                                    "skipped: too far from center",
                                )),
                            });
                            if let Some(ctx) = repaint_ctx.get() {
                                ctx.request_repaint();
                            }
                            continue;
                        }

                        let decode_priority = match job.priority {
                            LoadPriority::Urgent => loader::DecodePriority::Interactive,
                            LoadPriority::High => loader::DecodePriority::Interactive,
                            LoadPriority::Normal => loader::DecodePriority::Background,
                        };

                        let frame = loader::decode_full_frame_with_priority(
                            &job.path,
                            decode_priority,
                        );

                        // Re-check relevance after (potentially slow) decode.
                        let center = active_center.load(Ordering::Relaxed);
                        if job.index.abs_diff(center) > skip_radius + 2 {
                            continue;
                        }

                        if results_tx.send(LoadOutput {
                            index: job.index,
                            frame,
                        }).is_err() {
                            break; // Receiver dropped; exit worker loop.
                        }
                        if let Some(ctx) = repaint_ctx.get() {
                            ctx.request_repaint();
                        }
                    }
                });

            match spawn_result {
                Ok(handle) => {
                    worker_handles.push(handle);
                }
                Err(err) => {
                    log::warn!(
                        "[IMAGE-VIEWER] failed to spawn loader worker {}: {}",
                        worker_id,
                        err
                    );
                }
            }
        }

        Self {
            jobs_tx: Some(jobs_tx),
            bg_jobs_tx: Some(bg_jobs_tx),
            urgent_job,
            urgent_notify_tx: Some(urgent_notify_tx),
            results_rx,
            active_center,
            repaint_ctx,
            worker_handles,
        }
    }

    /// Provides the egui context so worker threads can trigger repaints
    /// when decode results are ready, avoiding UI polling.
    pub fn set_repaint_ctx(&self, ctx: egui::Context) {
        let _ = self.repaint_ctx.set(ctx);
    }

    /// Update the center index so workers can skip irrelevant jobs.
    pub fn set_center(&self, index: usize) {
        self.active_center.store(index, Ordering::Relaxed);
    }

    pub fn request(
        &self,
        index: usize,
        path: PathBuf,
        priority: LoadPriority,
    ) -> bool {
        let job = LoadJob {
            index,
            path,
            priority,
        };

        match job.priority {
            LoadPriority::Urgent => {
                if let Ok(mut slot) = self.urgent_job.lock() {
                    slot.replace(job);
                    // Wake a worker blocked in select! so it picks up the
                    // urgent job without waiting for a channel message.
                    if let Some(tx) = &self.urgent_notify_tx {
                        let _ = tx.try_send(());
                    }
                    true
                } else {
                    false
                }
            }
            LoadPriority::High => {
                self.jobs_tx.as_ref().map_or(false, |tx| tx.try_send(job).is_ok())
            }
            LoadPriority::Normal => {
                self.bg_jobs_tx.as_ref().map_or(false, |tx| tx.try_send(job).is_ok())
            }
        }
    }

    pub fn drain_results(&self, max_items: usize) -> Vec<LoadOutput> {
        let mut out = Vec::new();
        for _ in 0..max_items {
            match self.results_rx.try_recv() {
                Ok(v) => out.push(v),
                Err(_) => break,
            }
        }
        out
    }
}

impl Drop for PrefetchEngine {
    fn drop(&mut self) {
        // Drop senders first so workers see the channel disconnect and
        // exit their select! loops. Workers will then run their thread-local
        // destructors (ComInitGuard → CoUninitialize + WIC factory release).
        //
        // We deliberately do NOT join the worker threads here because:
        // 1. join() blocks the calling thread (UI thread) until the worker
        //    finishes its current decode, which can take seconds for large
        //    images/GIFs — this causes eframe's shutdown to hang/crash.
        // 2. The ComInitGuard thread-local handles COM/WIC cleanup
        //    automatically when the worker thread exits.
        // 3. If the process exits before workers finish, the OS reclaims
        //    all resources (handles, COM apartments, threads) anyway.
        self.jobs_tx.take();
        self.bg_jobs_tx.take();
        self.urgent_notify_tx.take();

        // Drop the JoinHandles without joining. The threads become
        // detached and will exit on their own once channels are
        // disconnected. Thread-local destructors handle cleanup.
        self.worker_handles.clear();
    }
}

