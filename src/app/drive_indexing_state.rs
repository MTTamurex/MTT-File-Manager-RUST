use crate::infrastructure::drive_indexing::{
    DriveIndexingAvailability, DriveIndexingError, DriveIndexingErrorKind, DriveIndexingView,
    SharedDriveIndexing,
};
use crate::workers::drive_indexing_worker::{
    DriveIndexingChange, DriveIndexingWorker, DriveIndexingWriteOutcome,
};
use std::collections::BTreeMap;

mod integration;

pub struct DriveIndexingState {
    pub view: DriveIndexingView,
    pub pending: BTreeMap<char, DriveIndexingChange>,
    pub errors: BTreeMap<char, DriveIndexingError>,
    pub last_error: Option<DriveIndexingError>,
    pub refresh_error: Option<DriveIndexingError>,
    pub shared: SharedDriveIndexing,
    worker: Option<DriveIndexingWorker>,
    next_request_id: u64,
    last_observed_cache: Option<String>,
}

#[derive(Default)]
pub struct DriveIndexingPoll {
    pub cache_changed: bool,
    pub policy_changed: bool,
    pub changed_drive_mask: u32,
}

impl DriveIndexingState {
    pub fn start(shared: SharedDriveIndexing, ctx: eframe::egui::Context) -> Self {
        let mut state = Self::from_shared(shared);
        match DriveIndexingWorker::start(state.shared.clone(), ctx) {
            Ok(worker) => state.worker = Some(worker),
            Err(_) => {
                state.last_error = Some(DriveIndexingError::transport(
                    "Cannot start drive configuration worker",
                ))
            }
        }
        state
    }

    fn from_shared(shared: SharedDriveIndexing) -> Self {
        let view = shared.snapshot();
        Self {
            last_observed_cache: view.cache_json(),
            view,
            shared,
            worker: None,
            next_request_id: 0,
            pending: BTreeMap::new(),
            errors: BTreeMap::new(),
            last_error: None,
            refresh_error: None,
        }
    }

    pub fn request_refresh(&mut self) {
        if let Some(worker) = &self.worker {
            if let Err(error) = worker.refresh() {
                self.last_error = Some(error);
            }
        }
    }

    pub fn request_set(&mut self, letter: char, enabled: bool) -> Result<bool, DriveIndexingError> {
        let worker = self.worker.as_ref().ok_or_else(|| {
            DriveIndexingError::transport("Drive configuration worker unavailable")
        })?;
        // Queueing does not mutate the confirmed configuration or its offline cache.
        let sender = |change| worker.set(change);
        let result = queue_change(
            &self.shared.snapshot(),
            &mut self.pending,
            &mut self.next_request_id,
            letter,
            enabled,
            sender,
        );
        if matches!(result, Ok(true)) {
            self.errors.remove(&letter.to_ascii_uppercase());
        }
        result
    }

    pub fn poll(&mut self) -> DriveIndexingPoll {
        if let Some(worker) = &self.worker {
            let status = worker.status();
            let running = worker.is_running();
            self.apply_status(status);
            if !running {
                self.last_error = Some(DriveIndexingError::transport(
                    "Drive configuration worker stopped",
                ));
            }
        }
        let view = self.shared.snapshot();
        let cache = view.cache_json();
        let policy_changed = self.view.generation != view.generation;
        let mut changed_drive_mask = self.view.disabled_mask() ^ view.disabled_mask();
        if policy_changed && changed_drive_mask == 0 {
            // Multiple flips may finish with the same mask between UI polls.
            // Their index generations still invalidate previously cached sizes.
            changed_drive_mask = (1 << 26) - 1;
        }
        let result = DriveIndexingPoll {
            policy_changed,
            changed_drive_mask,
            cache_changed: cache != self.last_observed_cache,
        };
        self.last_observed_cache = cache;
        self.view = view;
        result
    }

    fn apply_status(
        &mut self,
        status: crate::workers::drive_indexing_worker::DriveIndexingWorkerStatus,
    ) {
        self.refresh_error = status.refresh_error;
        self.pending.retain(|letter, change| {
            let Some(operation) = status
                .operations
                .get(letter)
                .filter(|operation| operation.request_id == change.request_id)
            else {
                return true;
            };
            match &operation.outcome {
                DriveIndexingWriteOutcome::Queued | DriveIndexingWriteOutcome::Pending => true,
                DriveIndexingWriteOutcome::Confirmed { enabled } => {
                    if *enabled != change.enabled {
                        self.errors.insert(
                            *letter,
                            DriveIndexingError::new(
                                DriveIndexingErrorKind::Service,
                                "Drive configuration changed while reconciling the request",
                            ),
                        );
                    } else {
                        self.errors.remove(letter);
                    }
                    false
                }
                DriveIndexingWriteOutcome::Failed(error) => {
                    self.errors.insert(*letter, error.clone());
                    false
                }
            }
        });
    }

    pub fn cache_json(&self) -> Option<String> {
        self.shared.snapshot().cache_json()
    }
}

fn queue_change(
    view: &DriveIndexingView,
    pending: &mut BTreeMap<char, DriveIndexingChange>,
    next_id: &mut u64,
    letter: char,
    enabled: bool,
    submit: impl FnOnce(DriveIndexingChange) -> Result<(), DriveIndexingError>,
) -> Result<bool, DriveIndexingError> {
    if !letter.is_ascii_alphabetic() {
        return Err(DriveIndexingError::new(
            DriveIndexingErrorKind::InvalidDrive,
            "Invalid drive letter",
        ));
    }
    let letter = letter.to_ascii_uppercase();
    if pending.contains_key(&letter) {
        return Err(DriveIndexingError::new(
            DriveIndexingErrorKind::Busy,
            "Drive configuration change is still pending",
        ));
    }
    if view.availability != DriveIndexingAvailability::Supported {
        return Err(DriveIndexingError::new(
            match view.availability {
                DriveIndexingAvailability::Unsupported => DriveIndexingErrorKind::Unsupported,
                DriveIndexingAvailability::AccessDenied => DriveIndexingErrorKind::AccessDenied,
                DriveIndexingAvailability::Offline => DriveIndexingErrorKind::Transport,
                _ => DriveIndexingErrorKind::Service,
            },
            "Drive configuration is not available for editing",
        ));
    }
    if view.is_enabled(letter) == enabled {
        return Ok(false);
    }
    *next_id = next_id.wrapping_add(1).max(1);
    let change = DriveIndexingChange {
        request_id: *next_id,
        drive_letter: letter,
        enabled,
    };
    submit(change.clone())?;
    pending.insert(letter, change);
    Ok(true)
}

#[cfg(test)]
mod tests;
