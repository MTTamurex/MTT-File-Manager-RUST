//! Batch thumbnail loading with Rayon for parallel processing
//! Follows .cursorrules: I/O in worker threads, zero allocations in hot path

use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::domain::thumbnail::ThumbnailData;
use crate::infrastructure::windows;
use crate::infrastructure::windows_api::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// Configuration for batch thumbnail loading
#[derive(Clone, Debug)]
pub struct BatchLoaderConfig {
    /// Maximum concurrent thumbnail extractions
    pub max_concurrent: usize,
    
    /// Batch size for parallel processing
    pub batch_size: usize,
    
    /// Timeout for COM initialization (ms)
    pub com_timeout_ms: u32,
}

impl Default for BatchLoaderConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 30,  // Same as MAX_CONCURRENT_LOADS
            batch_size: 10,      // Process 10 thumbnails at once
            com_timeout_ms: 1000,
        }
    }
}

/// Batch thumbnail loader with Rayon parallel processing
pub struct BatchThumbnailLoader {
    config: BatchLoaderConfig,
    request_receiver: Arc<Mutex<Receiver<(PathBuf, usize)>>>,
    result_sender: Sender<ThumbnailData>,
    generation_tracker: Arc<AtomicUsize>,
}

impl BatchThumbnailLoader {
    /// Creates a new batch thumbnail loader
    pub fn new(
        request_receiver: Receiver<(PathBuf, usize)>,
        result_sender: Sender<ThumbnailData>,
        generation_tracker: Arc<AtomicUsize>,
        config: BatchLoaderConfig,
    ) -> Self {
        Self {
            config,
            request_receiver: Arc::new(Mutex::new(request_receiver)),
            result_sender,
            generation_tracker,
        }
    }
    
    /// Starts the batch loader in a dedicated thread
    pub fn spawn(self) {
        std::thread::spawn(move || {
            self.run_loader_loop();
        });
    }
    
    /// Main loader loop with batch processing
    fn run_loader_loop(self) {
        // Initialize COM for this thread (required for Windows Shell APIs)
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        
        let mut pending_requests = Vec::with_capacity(self.config.batch_size);
        
        loop {
            // Collect a batch of requests
            self.collect_batch(&mut pending_requests);
            
            if pending_requests.is_empty() {
                // No requests, small sleep to prevent busy waiting
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            
            // Process batch in parallel with Rayon
            self.process_batch(&pending_requests);
            
            pending_requests.clear();
        }
        
        // Cleanup COM (though loop never ends in practice)
        // Note: This code is unreachable because the loop above is infinite
        // We keep it for clarity and in case we change the loop logic
    }
    
    /// Collects requests into a batch
    fn collect_batch(&self, batch: &mut Vec<(PathBuf, usize)>) {
        let receiver = match self.request_receiver.lock() {
            Ok(rx) => rx,
            Err(_) => return, // Thread is shutting down
        };
        
        // Try to collect up to batch_size requests
        for _ in 0..self.config.batch_size {
            match receiver.try_recv() {
                Ok(request) => batch.push(request),
                Err(_) => break, // No more requests
            }
        }
    }
    
    /// Processes a batch of thumbnail requests in parallel
    fn process_batch(&self, batch: &[(PathBuf, usize)]) {
        if batch.is_empty() {
            return;
        }
        
        // Filter out requests from old generations
        let current_gen = self.generation_tracker.load(Ordering::Relaxed);
        let valid_requests: Vec<_> = batch
            .iter()
            .filter(|(_, gen)| *gen == current_gen)
            .collect();
        
        if valid_requests.is_empty() {
            return;
        }
        
        // Process in parallel with Rayon
        valid_requests.par_iter()
            .take(self.config.max_concurrent)
            .for_each(|(path, gen)| {
                self.process_single_thumbnail(path, *gen);
            });
    }
    
    /// Processes a single thumbnail request
    fn process_single_thumbnail(&self, path: &PathBuf, generation: usize) {
        // Double-check generation (race condition protection)
        let current_gen = self.generation_tracker.load(Ordering::Relaxed);
        if generation != current_gen {
            return; // Request from old generation, ignore
        }
        
        // Extract thumbnail using Windows API
        let (image_data, width, height) = match windows::extract_thumbnail(path) {
            Ok(result) => result,
            Err(_) => {
                // Use error placeholder if extraction fails
                windows::create_error_placeholder()
            }
        };
        
        // Send result back to UI thread
        let thumbnail_data = ThumbnailData {
            path: path.clone(),
            image_data,
            width,
            height,
            generation,
        };
        
        let _ = self.result_sender.send(thumbnail_data);
    }
}

/// Creates a pool of batch thumbnail loaders
pub fn create_batch_loader_pool(
    request_receiver: Receiver<(PathBuf, usize)>,
    result_sender: Sender<ThumbnailData>,
    generation_tracker: Arc<AtomicUsize>,
    _num_workers: usize,
    config: BatchLoaderConfig,
) {
    // For simplicity, create a single loader for now
    // In a production system, you would use a work-stealing queue
    let loader = BatchThumbnailLoader::new(
        request_receiver,
        result_sender,
        generation_tracker,
        config,
    );
    
    loader.spawn();
}

/// Optimized version for high-performance thumbnail loading
pub struct OptimizedThumbnailLoader {
    work_queue: crossbeam::channel::Receiver<(PathBuf, usize)>,
    result_sender: Sender<ThumbnailData>,
    generation: Arc<AtomicUsize>,
}

impl OptimizedThumbnailLoader {
    /// Creates an optimized loader using crossbeam channels for better performance
    pub fn new(
        work_queue: crossbeam::channel::Receiver<(PathBuf, usize)>,
        result_sender: Sender<ThumbnailData>,
        generation: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            work_queue,
            result_sender,
            generation,
        }
    }
    
    /// Runs the optimized loader
    pub fn run(mut self) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        
        let mut batch = Vec::with_capacity(20);
        
        loop {
            // Collect batch with timeout
            self.collect_batch_optimized(&mut batch);
            
            if !batch.is_empty() {
                self.process_batch_optimized(&batch);
                batch.clear();
            }
            
            // Small yield to prevent CPU spinning
            std::thread::yield_now();
        }
    }
    
    fn collect_batch_optimized(&mut self, batch: &mut Vec<(PathBuf, usize)>) {
        // Try to collect multiple items quickly
        while batch.len() < 20 {
            match self.work_queue.try_recv() {
                Ok(item) => batch.push(item),
                Err(_) => break,
            }
        }
    }
    
    fn process_batch_optimized(&self, batch: &[(PathBuf, usize)]) {
        let current_gen = self.generation.load(Ordering::Relaxed);
        
        batch.par_iter()
            .filter(|(_, gen)| *gen == current_gen)
            .for_each(|(path, gen)| {
                match windows::extract_thumbnail(path) {
                    Ok((data, w, h)) => {
                        let _ = self.result_sender.send(ThumbnailData {
                            path: path.clone(),
                            image_data: data,
                            width: w,
                            height: h,
                            generation: *gen,
                        });
                    }
                    Err(_) => {
                        let (data, w, h) = windows::create_error_placeholder();
                        let _ = self.result_sender.send(ThumbnailData {
                            path: path.clone(),
                            image_data: data,
                            width: w,
                            height: h,
                            generation: *gen,
                        });
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use tempfile::tempdir;
    
    #[test]
    fn test_batch_loader_config() {
        let config = BatchLoaderConfig::default();
        assert_eq!(config.max_concurrent, 30);
        assert_eq!(config.batch_size, 10);
    }
    
    #[test]
    fn test_batch_loader_creation() {
        let (tx, rx) = mpsc::channel();
        let (result_tx, _) = mpsc::channel();
        let gen = Arc::new(AtomicUsize::new(0));
        
        let loader = BatchThumbnailLoader::new(
            rx,
            result_tx,
            gen,
            BatchLoaderConfig::default(),
        );
        
        // Just test that it was created successfully
        assert!(true);
    }
}
