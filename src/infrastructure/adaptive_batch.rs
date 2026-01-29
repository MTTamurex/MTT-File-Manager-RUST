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
            (true, _) => 500,
            (false, Some(n)) if n <= 50 => n.max(MIN_BATCH_SIZE),
            (false, Some(n)) if n <= 200 => 50,
            (false, Some(n)) if n <= 1000 => 100,
            (false, Some(_)) => 150,
            (false, None) => 75,
        }
    }
}

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

    pub fn record_batch(&mut self, duration: Duration, items_processed: usize) {
        if items_processed == 0 {
            return;
        }
        self.batch_times.push(duration);

        if self.batch_times.len() > 5 {
            self.batch_times.remove(0);
        }

        if self.is_ssd {
            return;
        }

        let avg_time_per_item = self
            .batch_times
            .iter()
            .map(|d| d.as_micros() as f64)
            .sum::<f64>()
            / (items_processed as f64 * self.batch_times.len() as f64);

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
}
