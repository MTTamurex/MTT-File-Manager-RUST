use crate::image_viewer::loader;
use crossbeam_channel::{Receiver, Sender};
use std::collections::HashMap;
use std::hash::Hash;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadKind {
    Preview,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LoadPriority {
    High,
    Normal,
}

#[derive(Debug)]
pub struct LoadOutput {
    pub sequence: u64,
    pub index: usize,
    pub kind: LoadKind,
    pub frame: io::Result<loader::DecodedFrame>,
    pub decode_us: u64,
}

#[derive(Clone, Debug)]
struct LoadJob {
    sequence: u64,
    index: usize,
    path: PathBuf,
    kind: LoadKind,
    priority: LoadPriority,
}

#[derive(Default)]
struct CachedImageLevels {
    preview: Option<Arc<loader::DecodedFrame>>,
    full: Option<Arc<loader::DecodedFrame>>,
}

pub struct WindowCache {
    radius: usize,
    items: HashMap<usize, CachedImageLevels>,
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

    pub fn has(&self, index: usize, kind: LoadKind) -> bool {
        self.items
            .get(&index)
            .is_some_and(|entry| match kind {
                LoadKind::Preview => entry.preview.is_some(),
                LoadKind::Full => entry.full.is_some(),
            })
    }

    pub fn put(&mut self, index: usize, kind: LoadKind, frame: Arc<loader::DecodedFrame>) {
        let entry = self.items.entry(index).or_default();
        match kind {
            LoadKind::Preview => entry.preview = Some(frame),
            LoadKind::Full => entry.full = Some(frame),
        }
    }

    pub fn get_best(&self, index: usize) -> Option<(LoadKind, Arc<loader::DecodedFrame>)> {
        let entry = self.items.get(&index)?;
        if let Some(frame) = &entry.full {
            return Some((LoadKind::Full, Arc::clone(frame)));
        }
        entry
            .preview
            .as_ref()
            .map(|frame| (LoadKind::Preview, Arc::clone(frame)))
    }

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
    high_jobs_tx: Sender<LoadJob>,
    normal_jobs_tx: Sender<LoadJob>,
    results_rx: Receiver<LoadOutput>,
    active_sequence: Arc<AtomicU64>,
}

impl PrefetchEngine {
    pub fn new(worker_count: usize, max_queue: usize) -> Self {
        let worker_count = worker_count.clamp(1, 6);
        let max_queue = max_queue.max(8);
        let high_queue = (max_queue / 2).max(4);

        let (high_jobs_tx, high_jobs_rx) = crossbeam_channel::bounded::<LoadJob>(high_queue);
        let (normal_jobs_tx, normal_jobs_rx) = crossbeam_channel::bounded::<LoadJob>(max_queue);
        let (results_tx, results_rx) = crossbeam_channel::unbounded::<LoadOutput>();
        let active_sequence = Arc::new(AtomicU64::new(0));

        for worker_id in 0..worker_count {
            let high_jobs_rx = high_jobs_rx.clone();
            let normal_jobs_rx = normal_jobs_rx.clone();
            let results_tx = results_tx.clone();
            let active_sequence = Arc::clone(&active_sequence);
            let spawn_result = std::thread::Builder::new()
                .name(format!("image-viewer-loader-{}", worker_id))
                .spawn(move || {
                    loop {
                        let job = match high_jobs_rx.try_recv() {
                            Ok(job) => job,
                            Err(crossbeam_channel::TryRecvError::Disconnected) => break,
                            Err(crossbeam_channel::TryRecvError::Empty) => {
                                crossbeam_channel::select! {
                                    recv(high_jobs_rx) -> recv_res => match recv_res {
                                        Ok(job) => job,
                                        Err(_) => break,
                                    },
                                    recv(normal_jobs_rx) -> recv_res => match recv_res {
                                        Ok(job) => job,
                                        Err(_) => break,
                                    }
                                }
                            }
                        };

                        if job.sequence != active_sequence.load(Ordering::Relaxed) {
                            continue;
                        }

                        let started = std::time::Instant::now();
                        let decode_priority = match job.priority {
                            LoadPriority::High => loader::DecodePriority::Interactive,
                            LoadPriority::Normal => loader::DecodePriority::Background,
                        };

                        let preview_side = match job.priority {
                            LoadPriority::High => 1280,
                            LoadPriority::Normal => 768,
                        };

                        let frame = match job.kind {
                            LoadKind::Preview => loader::decode_preview_frame_with_priority(
                                &job.path,
                                preview_side,
                                decode_priority,
                            ),
                            LoadKind::Full => loader::decode_full_frame_with_priority(
                                &job.path,
                                decode_priority,
                            ),
                        };

                        if job.sequence != active_sequence.load(Ordering::Relaxed) {
                            continue;
                        }

                        let elapsed = started.elapsed().as_micros() as u64;
                        let _ = results_tx.send(LoadOutput {
                            sequence: job.sequence,
                            index: job.index,
                            kind: job.kind,
                            frame,
                            decode_us: elapsed,
                        });
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
            high_jobs_tx,
            normal_jobs_tx,
            results_rx,
            active_sequence,
        }
    }

    pub fn set_active_sequence(&self, sequence: u64) {
        self.active_sequence.store(sequence, Ordering::Relaxed);
    }

    pub fn request(
        &self,
        sequence: u64,
        index: usize,
        path: PathBuf,
        kind: LoadKind,
        priority: LoadPriority,
    ) -> bool {
        let job = LoadJob {
            sequence,
            index,
            path,
            kind,
            priority,
        };

        match job.priority {
            LoadPriority::High => self.high_jobs_tx.try_send(job).is_ok(),
            LoadPriority::Normal => self.normal_jobs_tx.try_send(job).is_ok(),
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

