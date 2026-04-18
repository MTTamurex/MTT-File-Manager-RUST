use std::collections::VecDeque;
use std::time::Duration;

const MIN_BATCH_SIZE: usize = 25;
const MAX_BATCH_SIZE: usize = 1000;
const TARGET_BATCH_MS: u64 = 50;

pub struct AdaptiveBatchConfig {
    pub is_ssd: bool,
    pub total_items: Option<usize>,
}

impl AdaptiveBatchConfig {
    pub fn initial_batch_size(&self) -> usize {
        match (self.is_ssd, self.total_items) {
            // Keep SSD/NVMe responsive: smaller first batches improve time-to-first-items.
            (true, Some(n)) if n <= 80 => n.max(MIN_BATCH_SIZE),
            (true, _) => 250,
            (false, Some(n)) if n <= 50 => n.max(MIN_BATCH_SIZE),
            (false, Some(n)) if n <= 200 => 50,
            (false, Some(n)) if n <= 1000 => 100,
            (false, Some(_)) => 150,
            (false, None) => 75,
        }
    }
}

/// Per-item timing sample: how long a batch took and how many items it contained.
struct BatchSample {
    duration: Duration,
    items: usize,
}

pub struct AdaptiveBatchTracker {
    is_ssd: bool,
    samples: VecDeque<BatchSample>,
    current_batch_size: usize,
}

impl AdaptiveBatchTracker {
    pub fn new(config: AdaptiveBatchConfig) -> Self {
        Self {
            is_ssd: config.is_ssd,
            samples: VecDeque::with_capacity(6),
            current_batch_size: config.initial_batch_size(),
        }
    }

    pub fn record_batch(&mut self, duration: Duration, items_processed: usize) {
        if items_processed == 0 {
            return;
        }
        self.samples.push_back(BatchSample { duration, items: items_processed });

        if self.samples.len() > 5 {
            self.samples.pop_front();
        }

        if self.is_ssd {
            return;
        }

        // Compute weighted average: total_time / total_items across recent samples.
        let total_micros: f64 = self.samples.iter().map(|s| s.duration.as_micros() as f64).sum();
        let total_items: f64 = self.samples.iter().map(|s| s.items as f64).sum();

        let avg_time_per_item = total_micros / total_items;

        if avg_time_per_item <= 0.0 {
            return;
        }

        let target_items = (TARGET_BATCH_MS as f64 * 1000.0 / avg_time_per_item) as usize;
        let new_size = (self.current_batch_size + target_items) / 2;
        self.current_batch_size = new_size.clamp(MIN_BATCH_SIZE, MAX_BATCH_SIZE);
    }

    pub fn batch_size(&self) -> usize {
        self.current_batch_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_batch_size_hdd_small() {
        let config = AdaptiveBatchConfig {
            is_ssd: false,
            total_items: Some(40),
        };
        assert_eq!(config.initial_batch_size(), 40);
    }

    #[test]
    fn initial_batch_size_ssd_default() {
        let config = AdaptiveBatchConfig {
            is_ssd: true,
            total_items: Some(5000),
        };
        assert_eq!(config.initial_batch_size(), 250);
    }
}
