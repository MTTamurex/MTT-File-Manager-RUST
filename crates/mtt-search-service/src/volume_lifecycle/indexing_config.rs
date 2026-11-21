use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use mtt_search_protocol::{DriveIndexingEntry, SearchResponse};

use crate::index_db::IndexDb;
use crate::usn_journal::DiscoveredVolume;

use super::{VolumeLifecycle, VolumeRuntime};

#[derive(Default)]
pub(super) struct DriveConfigState {
    entries: HashMap<char, bool>,
    inventory: HashSet<char>,
    discovery_complete: bool,
    revision: u64,
    // Retain ownership after toggles and PnP disconnects until work has drained.
    pub(super) draining: HashMap<char, Arc<VolumeRuntime>>,
}

impl DriveConfigState {
    pub(super) fn matches_revision(&self, revision: u64) -> bool {
        self.revision == revision
    }

    pub(super) fn runtime_for_identity(
        &self,
        runtimes: &HashMap<char, Arc<VolumeRuntime>>,
        volume_id: &str,
        volume_serial: u32,
    ) -> Option<Arc<VolumeRuntime>> {
        runtimes
            .values()
            .chain(self.draining.values())
            .find(|runtime| {
                super::volume_ids_match(&runtime.volume_id, volume_id)
                    && runtime.volume_serial == volume_serial
            })
            .cloned()
    }

    pub(super) fn invalidate_discovery(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.discovery_complete = false;
    }

    pub(super) fn is_enabled(&self, letter: char) -> bool {
        self.entries.get(&letter).copied().unwrap_or(true)
    }

    pub(super) fn disabled_mask(&self) -> u32 {
        self.entries.iter().fold(0, |mask, (&letter, &enabled)| {
            if enabled {
                mask
            } else {
                mask | (1 << (letter as u32 - 'A' as u32))
            }
        })
    }
}

impl VolumeLifecycle {
    /// Startup-only: failure prevents discovery and indexer admission.
    pub(crate) fn initialize_drive_config(&self, db: Arc<IndexDb>) -> Result<(), String> {
        let entries = db.load_drive_indexing_config()?;
        self.config_db
            .set(db)
            .map_err(|_| "Drive configuration already initialized")?;
        self.drive_config.write().entries = entries;
        Ok(())
    }

    pub(crate) fn is_indexing_enabled(&self, letter: char) -> bool {
        self.drive_config
            .read()
            .is_enabled(letter.to_ascii_uppercase())
    }

    pub(crate) fn set_drive_indexing(&self, letter: char, enabled: bool) -> Result<(), String> {
        if !letter.is_ascii_alphabetic() {
            return Err("drive letter must be an ASCII letter".into());
        }
        let letter = letter.to_ascii_uppercase();
        let _update = self.config_updates.lock();
        let db = self
            .config_db
            .get()
            .ok_or("Drive configuration unavailable")?;

        // No registry/configuration locks during SQLite I/O. The commit precedes
        // policy application; the handler must finish applying even if IPC closes.
        db.set_drive_indexing(letter, enabled)?;
        {
            let mut config = self.drive_config.write();
            let runtimes = self.runtimes.lock();
            if config.is_enabled(letter) != enabled {
                config.invalidate_discovery();
            }
            config.entries.insert(letter, enabled);
            if !enabled {
                if let Some(runtime) = runtimes.get(&letter) {
                    let worker = runtime.worker.lock();
                    runtime.usage.begin_indexing_disable();
                    if let Some(worker) = worker.as_ref() {
                        worker.stop.request_stop();
                    }
                    config.draining.insert(letter, runtime.clone());
                    self.remove_index_if_owned(runtime);
                }
                self.indexing_progress.clear(letter);
            }
        }
        self.wake_discovery();
        Ok(())
    }

    pub(super) fn retire_indexing_drains(&self) {
        let mut config = self.drive_config.write();
        let before = config.draining.len();
        config.draining.retain(|_, runtime| {
            let worker = runtime.worker.lock();
            worker.is_some() || !runtime.usage.is_idle()
        });
        if config.draining.len() != before {
            // This discovery may have skipped the draining letter.
            config.invalidate_discovery();
            self.wake_discovery();
        }
    }

    /// Inventory is updated by discovery, never by an IPC handler's filesystem I/O.
    pub(crate) fn drive_config_revision(&self) -> u64 {
        self.drive_config.read().revision
    }

    pub(crate) fn record_drive_discovery(
        &self,
        discovered: &[DiscoveredVolume],
        revision: u64,
    ) -> bool {
        let mut config = self.drive_config.write();
        if config.revision != revision {
            // A toggle raced this pass. Its wake schedules a fresh discovery;
            // stale inventory cannot attest that no enabled volumes are pending.
            return false;
        }
        let runtimes = self.runtimes.lock();
        let mut inventory: HashSet<char> = discovered
            .iter()
            .filter(|volume| !volume.volume_id.is_empty())
            .map(|volume| volume.drive_letter)
            .collect();
        // Discovery deliberately omits maintenance-suspended volumes. Keep their
        // known coverage until lifecycle reconciliation can resume discovery.
        inventory.extend(
            runtimes
                .iter()
                .filter_map(|(&letter, runtime)| runtime.usage.is_removing().then_some(letter)),
        );
        config.inventory = inventory;
        config.discovery_complete = true;
        true
    }

    pub(crate) fn drive_indexing_config_response(&self) -> SearchResponse {
        let config = self.drive_config.read();
        let mut drives: BTreeMap<char, bool> = config
            .inventory
            .iter()
            .map(|&letter| (letter, config.is_enabled(letter)))
            .collect();
        drives.extend(
            config
                .entries
                .iter()
                .map(|(&letter, &enabled)| (letter, enabled)),
        );
        let mut enabled_service_volumes: Vec<char> = config
            .inventory
            .iter()
            .copied()
            .filter(|&letter| config.is_enabled(letter))
            .collect();
        enabled_service_volumes.sort_unstable();
        SearchResponse::DriveIndexingConfig {
            drives: drives
                .into_iter()
                .map(|(drive_letter, enabled)| DriveIndexingEntry {
                    drive_letter,
                    enabled,
                })
                .collect(),
            service_discovery_complete: config.discovery_complete,
            enabled_service_volumes,
        }
    }

    pub(crate) fn filter_disabled_status(&self, status: &mut mtt_search_protocol::IndexStatusInfo) {
        let config = self.drive_config.read();
        status
            .volumes
            .retain(|volume| config.is_enabled(volume.drive_letter));
        status.total_files_indexed = status
            .volumes
            .iter()
            .map(|volume| volume.files_indexed)
            .sum();
    }
}

#[cfg(test)]
#[path = "indexing_config_tests.rs"]
mod tests;
