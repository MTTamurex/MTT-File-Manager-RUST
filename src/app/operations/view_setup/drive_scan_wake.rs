//! Decides whether a finished drive-list scan has to wake the UI.
//!
//! The periodic fallback scan usually finds the same drives. Waking the UI for
//! it renders and presents an identical frame, which shows up as GPU activity
//! while the window sits idle. Quiet scans compare against the state captured
//! when they started and leave an unchanged result for the next scheduled pass.

use crate::app::drive_state::DriveScanResult;
use crate::domain::cloud_root::CloudRoot;
use crate::domain::network_location::NetworkLocation;

pub(super) struct DriveScanBaseline {
    disks: Vec<(String, String)>,
    cloud_roots: Vec<CloudRoot>,
    network_locations: Vec<NetworkLocation>,
}

impl DriveScanBaseline {
    pub(super) fn capture(state: &crate::app::drive_state::DriveState) -> Self {
        Self {
            disks: state.disks.clone(),
            cloud_roots: state.cloud_roots.clone(),
            network_locations: state.network_locations.clone(),
        }
    }

    fn matches(&self, scan: &DriveScanResult) -> bool {
        scan.disks == self.disks
            && scan.cloud_roots == self.cloud_roots
            && scan.network_locations == self.network_locations
    }
}

/// A loud scan (no baseline) always wakes the UI; a quiet one only on change.
pub(super) fn scan_wakes_ui(baseline: Option<&DriveScanBaseline>, scan: &DriveScanResult) -> bool {
    baseline.is_none_or(|baseline| !baseline.matches(scan))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(disks: Vec<(String, String)>) -> DriveScanResult {
        DriveScanResult {
            disks,
            cloud_roots: Vec::new(),
            network_locations: Vec::new(),
            unavailable_label_roots: Default::default(),
        }
    }

    fn baseline(disks: Vec<(String, String)>) -> DriveScanBaseline {
        DriveScanBaseline {
            disks,
            cloud_roots: Vec::new(),
            network_locations: Vec::new(),
        }
    }

    #[test]
    fn quiet_scan_wakes_only_when_the_drive_list_changed() {
        let disks = vec![("C:".to_string(), "Local Disk".to_string())];
        assert!(!scan_wakes_ui(
            Some(&baseline(disks.clone())),
            &scan(disks.clone())
        ));
        let mut changed = disks.clone();
        changed.push(("D:".to_string(), "Data".to_string()));
        assert!(scan_wakes_ui(Some(&baseline(disks)), &scan(changed)));
    }

    #[test]
    fn loud_scan_always_wakes() {
        assert!(scan_wakes_ui(None, &scan(Vec::new())));
    }
}
