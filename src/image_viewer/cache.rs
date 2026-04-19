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

/// Maximum memory budget for the image cache.
/// Frames are downscaled to [`loader::DISPLAY_CACHE_MAX_SIDE`] (4096 px on the
/// long edge) before being inserted, so a worst-case 4K-clamped RGBA frame
/// takes ~64 MB; 128 MB therefore comfortably holds the active window for
/// typical viewing while keeping process working set small.
const MAX_CACHE_BYTES: usize = 128 * 1024 * 1024;

pub struct WindowCache {
    radius: usize,
    items: HashMap<usize, Arc<loader::DecodedFrame>>,
    total_bytes: usize,
}

impl WindowCache {
    pub fn new(radius: usize) -> Self {
        Self {
            radius,
            items: HashMap::new(),
            total_bytes: 0,
        }
    }

    pub fn radius(&self) -> usize {
        self.radius
    }

    pub fn has(&self, index: usize) -> bool {
        self.items.contains_key(&index)
    }

    pub fn put(&mut self, index: usize, frame: Arc<loader::DecodedFrame>) {
        let new_bytes = frame.rgba.len();
        if let Some(old) = self.items.insert(index, frame) {
            self.total_bytes = self.total_bytes.saturating_sub(old.rgba.len());
        }
        self.total_bytes += new_bytes;
    }

    pub fn get(&self, index: usize) -> Option<Arc<loader::DecodedFrame>> {
        self.items.get(&index).map(Arc::clone)
    }

    pub fn retain_window(&mut self, center: usize, total_len: usize) {
        if total_len == 0 {
            self.items.clear();
            self.total_bytes = 0;
            return;
        }

        let min_idx = center.saturating_sub(self.radius);
        let max_idx = (center + self.radius).min(total_len.saturating_sub(1));
        let evict_bytes: usize = self
            .items
            .iter()
            .filter(|(&idx, _)| idx < min_idx || idx > max_idx)
            .map(|(_, frame)| frame.rgba.len())
            .sum();
        self.items
            .retain(|&idx, _| idx >= min_idx && idx <= max_idx);
        self.total_bytes = self.total_bytes.saturating_sub(evict_bytes);
    }

    /// Evicts entries from the furthest indices until under budget.
    pub fn evict_over_budget(&mut self, center: usize) {
        while self.total_bytes > MAX_CACHE_BYTES {
            let Some(&idx) = self.items.keys().max_by_key(|&&k| k.abs_diff(center)) else {
                break;
            };

            if let Some(frame) = self.items.remove(&idx) {
                self.total_bytes = self.total_bytes.saturating_sub(frame.rgba.len());
            } else {
                break;
            }
        }
    }
}

pub struct PrefetchEngine {
    jobs_tx: Sender<LoadJob>,
    bg_jobs_tx: Sender<LoadJob>,
    urgent_job: Arc<std::sync::Mutex<Option<LoadJob>>>,
    urgent_notify_tx: Sender<()>,
    results_rx: Receiver<LoadOutput>,
    active_center: Arc<AtomicUsize>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
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

            if let Err(err) = spawn_result {
                log::warn!(
                    "[IMAGE-VIEWER] failed to spawn loader worker {}: {}",
                    worker_id,
                    err
                );
            }
        }

        Self {
            jobs_tx,
            bg_jobs_tx,
            urgent_job,
            urgent_notify_tx,
            results_rx,
            active_center,
            repaint_ctx,
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
                    let _ = self.urgent_notify_tx.try_send(());
                    true
                } else {
                    false
                }
            }
            LoadPriority::High => self.jobs_tx.try_send(job).is_ok(),
            LoadPriority::Normal => self.bg_jobs_tx.try_send(job).is_ok(),
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

