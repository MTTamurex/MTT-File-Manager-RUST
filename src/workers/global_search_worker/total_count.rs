use super::*;

pub(super) struct CountTask {
    pub sender: Sender<GlobalSearchResponse>,
    pub ctx: eframe::egui::Context,
    pub query: String,
    pub local_total_matches: u32,
    pub generation: u64,
    pub active_generation: Arc<AtomicU64>,
    pub in_flight: Arc<AtomicBool>,
    pub policy: SharedDriveIndexing,
    pub indexing_generation: u64,
}

impl CountTask {
    fn is_current(&self) -> bool {
        self.active_generation.load(Ordering::Relaxed) == self.generation
            && self.policy.generation() == self.indexing_generation
    }

    fn load(&self) -> Result<Option<u32>, String> {
        let mut offset = 0u32;
        loop {
            if !self.is_current() {
                return Ok(None);
            }
            let page = query_service_with_retry(&self.query, offset, 1_000)?;
            let count = page.items.len() as u32;
            if !page.has_more {
                return Ok(Some(
                    page.total_matches.unwrap_or(offset.saturating_add(count)),
                ));
            }
            if count == 0 {
                return Ok(Some(offset));
            }
            offset = offset.saturating_add(count);
        }
    }

    pub fn spawn(self) {
        if self.in_flight.swap(true, Ordering::AcqRel) {
            return;
        }
        let in_flight = self.in_flight.clone();
        let result = crate::spawn_named("global-search-total-count", move || {
            struct Guard(Arc<AtomicBool>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _guard = Guard(self.in_flight.clone());
            match self.load() {
                Ok(Some(total)) if self.is_current() => {
                    let _ = self.sender.send(GlobalSearchResponse::TotalCount {
                        query: self.query.clone(),
                        total_matches: total.saturating_add(self.local_total_matches),
                        indexing_generation: self.indexing_generation,
                    });
                    self.ctx.request_repaint();
                }
                Err(error) if self.is_current() => {
                    log::debug!("[GLOBAL-SEARCH] Exact total count unavailable: {}", error);
                }
                _ => {}
            }
        });
        if let Err(error) = result {
            in_flight.store(false, Ordering::Release);
            log::error!(
                "[GLOBAL-SEARCH] Failed to spawn total count task: {}",
                error
            );
        }
    }
}

#[cfg(test)]
#[path = "total_count_tests.rs"]
mod tests;
