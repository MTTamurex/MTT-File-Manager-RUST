//! Small, per-app immutable policy snapshots shared with background consumers.
//! This module does not depend on UI state and performs no IPC or disk I/O.
use mtt_search_protocol::{DriveIndexingEntry, SearchResponse};
use parking_lot::RwLock;
use std::path::{Component, Path, Prefix};
use std::sync::Arc;

mod error;
pub use error::{DriveIndexingError, DriveIndexingErrorKind};

pub const CACHE_PREFERENCE_KEY: &str = "drive_indexing_config_cache";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveIndexingSnapshot {
    pub drives: Vec<DriveIndexingEntry>,
    pub service_discovery_complete: bool,
    pub enabled_service_volumes: Vec<char>,
}

impl DriveIndexingSnapshot {
    pub fn new(
        mut drives: Vec<DriveIndexingEntry>,
        service_discovery_complete: bool,
        mut enabled_service_volumes: Vec<char>,
    ) -> Result<Self, DriveIndexingError> {
        SearchResponse::DriveIndexingConfig {
            drives: drives.clone(),
            service_discovery_complete,
            enabled_service_volumes: enabled_service_volumes.clone(),
        }
        .validate()
        .map_err(DriveIndexingError::protocol)?;
        drives.sort_by_key(|entry| entry.drive_letter);
        enabled_service_volumes.sort_unstable();
        Ok(Self {
            drives,
            service_discovery_complete,
            enabled_service_volumes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveIndexingAvailability {
    Unknown,
    Supported,
    Unsupported,
    Offline,
    AccessDenied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveIndexingView {
    /// Last confirmed entries, including the offline cache; never optimistic.
    pub drives: Vec<DriveIndexingEntry>,
    pub has_confirmed_config: bool,
    pub availability: DriveIndexingAvailability,
    /// Remember a confirmed legacy service even if a later probe loses transport.
    pub legacy_service: bool,
    /// Only present while a current service query has succeeded.
    pub coverage: Option<DriveIndexingSnapshot>,
    /// Changes only when the effective indexing policy changes.
    pub generation: u64,
}

impl DriveIndexingView {
    pub fn is_enabled(&self, letter: char) -> bool {
        self.legacy_service
            || !self
                .drives
                .iter()
                .any(|entry| entry.drive_letter == letter.to_ascii_uppercase() && !entry.enabled)
    }

    pub fn cache_json(&self) -> Option<String> {
        self.has_confirmed_config
            .then(|| serde_json::to_string(&self.drives).ok())
            .flatten()
    }

    pub(crate) fn disabled_mask(&self) -> u32 {
        if self.legacy_service {
            return 0;
        }
        self.drives
            .iter()
            .filter(|entry| !entry.enabled)
            .fold(0, |mask, entry| {
                mask | (1 << (entry.drive_letter as u32 - 'A' as u32))
            })
    }
}

#[derive(Clone)]
pub struct SharedDriveIndexing(Arc<RwLock<DriveIndexingView>>);

impl SharedDriveIndexing {
    pub fn from_cache(json: Option<&str>) -> Self {
        let cached = json.and_then(|json| {
            let result = serde_json::from_str::<Vec<DriveIndexingEntry>>(json)
                .map_err(|_| DriveIndexingError::protocol("Invalid cached drive configuration"))
                .and_then(|drives| DriveIndexingSnapshot::new(drives, false, vec![]));
            if result.is_err() {
                log::warn!("[DRIVE-INDEXING] Ignoring invalid cached configuration");
            }
            result.ok()
        });
        Self(Arc::new(RwLock::new(DriveIndexingView {
            has_confirmed_config: cached.is_some(),
            drives: cached.map(|snapshot| snapshot.drives).unwrap_or_default(),
            availability: DriveIndexingAvailability::Unknown,
            legacy_service: false,
            coverage: None,
            generation: 0,
        })))
    }

    pub fn snapshot(&self) -> DriveIndexingView {
        self.0.read().clone()
    }

    pub fn generation(&self) -> u64 {
        self.0.read().generation
    }

    pub fn is_enabled(&self, letter: char) -> bool {
        self.0.read().is_enabled(letter)
    }

    pub fn is_path_enabled(&self, path: &Path) -> bool {
        path_drive_letter(path).is_none_or(|letter| self.is_enabled(letter))
    }

    pub(crate) fn confirm(&self, snapshot: DriveIndexingSnapshot) {
        let mut view = self.0.write();
        let before = view.disabled_mask();
        view.drives = snapshot.drives.clone();
        view.has_confirmed_config = true;
        view.coverage = Some(snapshot);
        view.availability = DriveIndexingAvailability::Supported;
        view.legacy_service = false;
        if before != view.disabled_mask() {
            view.generation = view.generation.wrapping_add(1);
        }
    }

    pub(crate) fn unavailable(&self, error: &DriveIndexingError) {
        let mut view = self.0.write();
        let before = view.disabled_mask();
        if error.kind == DriveIndexingErrorKind::Unsupported {
            view.legacy_service = true;
        }
        view.availability = match error.kind {
            DriveIndexingErrorKind::Unsupported => DriveIndexingAvailability::Unsupported,
            DriveIndexingErrorKind::AccessDenied => DriveIndexingAvailability::AccessDenied,
            DriveIndexingErrorKind::Transport | DriveIndexingErrorKind::Busy => {
                DriveIndexingAvailability::Offline
            }
            _ => DriveIndexingAvailability::Failed,
        };
        view.coverage = None;
        if before != view.disabled_mask() {
            view.generation = view.generation.wrapping_add(1);
        }
    }
}

pub fn path_drive_letter(path: &Path) -> Option<char> {
    match path.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some((letter as char).to_ascii_uppercase())
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests;
