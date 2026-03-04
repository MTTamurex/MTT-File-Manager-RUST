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
/// Full-resolution 4K images use ~33 MB each; 512 MB allows ~15 full-res entries.
const MAX_CACHE_BYTES: usize = 512 * 1024 * 1024;

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
        if self.total_bytes <= MAX_CACHE_BYTES {
            return;
        }

        let mut indices: Vec<usize> = self.items.keys().copied().collect();
        indices.sort_by(|a, b| b.abs_diff(center).cmp(&a.abs_diff(center)));

        for idx in indices {
            if self.total_bytes <= MAX_CACHE_BYTES {
                break;
            }

            if let Some(frame) = self.items.remove(&idx) {
                self.total_bytes = self.total_bytes.saturating_sub(frame.rgba.len());
            }
        }
    }
}

pub struct PrefetchEngine {
    jobs_tx: Sender<LoadJob>,
    bg_jobs_tx: Sender<LoadJob>,
    urgent_job: Arc<std::sync::Mutex<Option<LoadJob>>>,
    results_tx: Sender<LoadOutput>,
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
        let (results_tx, results_rx) = crossbeam_channel::unbounded::<LoadOutput>();
        let urgent_job = Arc::new(std::sync::Mutex::new(None));
        let active_center = Arc::new(AtomicUsize::new(0));
        let repaint_ctx = Arc::new(OnceLock::<egui::Context>::new());

        for worker_id in 0..worker_count {
            let jobs_rx = jobs_rx.clone();
            let bg_jobs_rx = bg_jobs_rx.clone();
            let urgent_job = Arc::clone(&urgent_job);
            let results_tx = results_tx.clone();
            let active_center = Arc::clone(&active_center);
            let repaint_ctx = Arc::clone(&repaint_ctx);
            let spawn_result = std::thread::Builder::new()
                .name(format!("image-viewer-loader-{}", worker_id))
                .spawn(move || {
                    loop {
                        let job = if let Ok(mut slot) = urgent_job.lock() {
                            if let Some(job) = slot.take() {
                                job
                            } else {
                                match jobs_rx.try_recv() {
                                    Ok(job) => job,
                                    Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                                    Err(crossbeam_channel::TryRecvError::Empty) => {
                                        crossbeam_channel::select! {
                                            recv(jobs_rx) -> recv_res => match recv_res {
                                                Ok(job) => job,
                                                Err(_) => break,
                                            },
                                            recv(bg_jobs_rx) -> recv_res => match recv_res {
                                                Ok(job) => job,
                                                Err(_) => break,
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            match jobs_rx.try_recv() {
                                Ok(job) => job,
                                Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                                Err(crossbeam_channel::TryRecvError::Empty) => {
                                    crossbeam_channel::select! {
                                        recv(jobs_rx) -> recv_res => match recv_res {
                                            Ok(job) => job,
                                            Err(_) => break,
                                        },
                                        recv(bg_jobs_rx) -> recv_res => match recv_res {
                                            Ok(job) => job,
                                            Err(_) => break,
                                        }
                                    }
                                }
                            }
                        };

                        // Skip jobs for images too far from the current view.
                        let center = active_center.load(Ordering::Relaxed);
                        if job.index.abs_diff(center) > skip_radius + 2 {
                            // Notify the UI that this job was skipped so it can
                            // clear the entry from requested_jobs and retry later.
                            let _ = results_tx.send(LoadOutput {
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

                        let _ = results_tx.send(LoadOutput {
                            index: job.index,
                            frame,
                        });
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
            results_tx,
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
                    if let Some(replaced) = slot.replace(job) {
                        let _ = self.results_tx.send(LoadOutput {
                            index: replaced.index,
                            frame: Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "replaced by newer urgent job",
                            )),
                        });
                    }
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

