use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct Metrics {
    decode_count: AtomicU64,
    decode_total_us: AtomicU64,
    upload_count: AtomicU64,
    upload_total_us: AtomicU64,
}

impl Metrics {
    pub fn record_decode_us(&self, elapsed_us: u64) {
        self.decode_count.fetch_add(1, Ordering::Relaxed);
        self.decode_total_us.fetch_add(elapsed_us, Ordering::Relaxed);
    }

    pub fn record_upload_us(&self, elapsed_us: u64) {
        self.upload_count.fetch_add(1, Ordering::Relaxed);
        self.upload_total_us.fetch_add(elapsed_us, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn decode_avg_ms(&self) -> f32 {
        let count = self.decode_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        let total_us = self.decode_total_us.load(Ordering::Relaxed);
        (total_us as f32 / count as f32) / 1000.0
    }

    #[allow(dead_code)]
    pub fn upload_avg_ms(&self) -> f32 {
        let count = self.upload_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        let total_us = self.upload_total_us.load(Ordering::Relaxed);
        (total_us as f32 / count as f32) / 1000.0
    }
}

