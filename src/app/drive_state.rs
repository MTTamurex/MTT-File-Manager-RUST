use crate::domain::cloud_root::CloudRoot;
use crate::domain::file_entry::DriveInfo;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

mod drive_info_refresh;
pub use drive_info_refresh::{
    merge_drive_info_query, DriveInfoRefreshEntry, DriveInfoRefreshResult, DriveInfoRefreshScope,
    DriveInfoRefreshTracker,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveScanResult {
    pub disks: Vec<(String, String)>,
    pub cloud_roots: Vec<CloudRoot>,
    pub unavailable_label_roots: HashSet<String>,
}

pub struct DriveState {
    pub disks: Vec<(String, String)>,
    pub cloud_roots: Vec<CloudRoot>,
    /// Deferred full drive/cloud detection from startup (delivers once, then set to None).
    pub cloud_root_rx: Option<Receiver<DriveScanResult>>,
    pub last_drive_refresh: Instant,
    pub last_drive_bitmask: u32,
    pub drive_scan_pending: bool,
    pub drive_scan_rx: Receiver<DriveScanResult>,
    pub drive_scan_tx: Sender<DriveScanResult>,
    pub drive_info_rx: Receiver<DriveInfoRefreshResult>,
    pub drive_info_tx: Sender<DriveInfoRefreshResult>,
    pub drive_info_cache: HashMap<String, DriveInfo>,
    pub drive_info_cache_epoch: u64,
    pub optimistically_hidden_drives: HashSet<String>,
    pub drive_info_refresh: DriveInfoRefreshTracker,
}

impl DriveState {
    pub fn cache_drive_info(&mut self, path: &str, info: DriveInfo) {
        self.drive_info_cache.insert(path.to_string(), info.clone());
        if let Some(root_key) = normalize_drive_root_key(path) {
            self.drive_info_cache.insert(root_key, info);
        }
        self.drive_info_cache_epoch = self.drive_info_cache_epoch.wrapping_add(1);
    }

    pub fn cached_drive_info(&self, path: &str) -> Option<DriveInfo> {
        self.drive_info_cache.get(path).cloned().or_else(|| {
            normalize_drive_root_key(path)
                .and_then(|root_key| self.drive_info_cache.get(&root_key).cloned())
        })
    }

    pub fn remove_cached_drive_info(&mut self, path: &str) {
        self.drive_info_cache.remove(path);
        if let Some(root_key) = normalize_drive_root_key(path) {
            self.drive_info_cache.remove(&root_key);
        }
        self.drive_info_cache_epoch = self.drive_info_cache_epoch.wrapping_add(1);
    }

    pub fn clear_cached_drive_info(&mut self) {
        self.drive_info_cache.clear();
        self.drive_info_cache_epoch = self.drive_info_cache_epoch.wrapping_add(1);
    }

    pub fn invalidate_drive_info_refreshes(&mut self) {
        self.drive_info_refresh.invalidate();
    }

    pub fn canonical_current_drive(&self, detected_drive: &str) -> Option<String> {
        let detected_key = normalize_drive_root_key(detected_drive)?;
        self.disks.iter().find_map(|(path, _)| {
            let current_key = normalize_drive_root_key(path)?;
            (current_key == detected_key).then_some(current_key)
        })
    }

    pub fn hide_drive_optimistically(&mut self, path: &str) -> bool {
        let Some(root_key) = normalize_drive_root_key(path) else {
            return false;
        };

        let newly_hidden = self.optimistically_hidden_drives.insert(root_key.clone());
        let old_len = self.disks.len();
        self.disks.retain(|(drive_path, _)| {
            normalize_drive_root_key(drive_path).as_ref() != Some(&root_key)
        });
        self.remove_cached_drive_info(path);
        newly_hidden || self.disks.len() != old_len
    }

    pub fn unhide_drive(&mut self, path: &str) {
        if let Some(root_key) = normalize_drive_root_key(path) {
            self.optimistically_hidden_drives.remove(&root_key);
        }
    }

    pub fn apply_optimistic_drive_filter(&mut self, scan_result: &mut DriveScanResult) {
        if self.optimistically_hidden_drives.is_empty() {
            return;
        }

        let hidden_drives: Vec<String> =
            self.optimistically_hidden_drives.iter().cloned().collect();
        for hidden_drive in hidden_drives {
            let still_reported = scan_result
                .disks
                .iter()
                .any(|(path, _)| normalize_drive_root_key(path).as_ref() == Some(&hidden_drive));

            if still_reported {
                scan_result.disks.retain(|(path, _)| {
                    normalize_drive_root_key(path).as_ref() != Some(&hidden_drive)
                });
            } else {
                self.optimistically_hidden_drives.remove(&hidden_drive);
            }
        }
    }

    pub fn preserve_labels_from_unavailable_queries(&self, scan_result: &mut DriveScanResult) {
        for (path, label) in &mut scan_result.disks {
            let Some(root_key) = normalize_drive_root_key(path) else {
                continue;
            };
            if !scan_result.unavailable_label_roots.contains(&root_key) {
                continue;
            }

            if let Some((_, known_label)) = self.disks.iter().find(|(known_path, _)| {
                normalize_drive_root_key(known_path).as_ref() == Some(&root_key)
            }) {
                *label = known_label.clone();
            }
        }
    }
}

pub fn normalize_drive_root_key(path: &str) -> Option<String> {
    let mut chars = path.chars();
    let drive = chars.next()?;
    if chars.next()? != ':' || !drive.is_ascii_alphabetic() {
        return None;
    }

    Some(format!("{}:\\", drive.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn test_drive_state(disks: Vec<(String, String)>) -> DriveState {
        let (_scan_tx, scan_rx) = mpsc::channel();
        let (drive_scan_tx, drive_scan_rx) = mpsc::channel();
        let (drive_info_tx, drive_info_rx) = mpsc::channel();

        DriveState {
            disks,
            cloud_roots: Vec::new(),
            cloud_root_rx: Some(scan_rx),
            last_drive_refresh: Instant::now(),
            last_drive_bitmask: 0,
            drive_scan_pending: false,
            drive_scan_rx,
            drive_scan_tx,
            drive_info_rx,
            drive_info_tx,
            drive_info_cache: HashMap::new(),
            drive_info_cache_epoch: 0,
            optimistically_hidden_drives: HashSet::new(),
            drive_info_refresh: DriveInfoRefreshTracker::new(Instant::now()),
        }
    }

    #[test]
    fn normalize_drive_root_key_accepts_drive_forms() {
        assert_eq!(normalize_drive_root_key("e:"), Some("E:\\".to_string()));
        assert_eq!(normalize_drive_root_key("E:\\"), Some("E:\\".to_string()));
        assert_eq!(
            normalize_drive_root_key("E:\\folder"),
            Some("E:\\".to_string())
        );
    }

    #[test]
    fn normalize_drive_root_key_rejects_non_drive_paths() {
        assert_eq!(normalize_drive_root_key("Este Computador"), None);
        assert_eq!(normalize_drive_root_key("\\\\server\\share"), None);
        assert_eq!(normalize_drive_root_key(""), None);
    }

    #[test]
    fn canonical_current_drive_rejects_stale_or_invalid_drives() {
        let state = test_drive_state(vec![("I:\\".to_string(), "DVD (I:)".to_string())]);

        assert_eq!(
            state.canonical_current_drive("i:"),
            Some("I:\\".to_string())
        );
        assert_eq!(state.canonical_current_drive("J:\\"), None);
        assert_eq!(state.canonical_current_drive("invalid"), None);
    }

    #[test]
    fn optimistic_drive_hide_filters_transient_scan_results() {
        let mut state = test_drive_state(vec![
            ("E:\\".to_string(), "ISO (E:)".to_string()),
            ("F:\\".to_string(), "Data (F:)".to_string()),
        ]);

        assert!(state.hide_drive_optimistically("E:\\"));
        assert_eq!(
            state.disks,
            vec![("F:\\".to_string(), "Data (F:)".to_string())]
        );

        let mut scan_result = DriveScanResult {
            disks: vec![
                ("E:\\".to_string(), "ISO (E:)".to_string()),
                ("F:\\".to_string(), "Data (F:)".to_string()),
            ],
            cloud_roots: Vec::new(),
            unavailable_label_roots: HashSet::new(),
        };
        state.apply_optimistic_drive_filter(&mut scan_result);

        assert_eq!(
            scan_result.disks,
            vec![("F:\\".to_string(), "Data (F:)".to_string())]
        );
        assert!(state.optimistically_hidden_drives.contains("E:\\"));

        let mut confirmed_removed = DriveScanResult {
            disks: vec![("F:\\".to_string(), "Data (F:)".to_string())],
            cloud_roots: Vec::new(),
            unavailable_label_roots: HashSet::new(),
        };
        state.apply_optimistic_drive_filter(&mut confirmed_removed);

        assert!(state.optimistically_hidden_drives.is_empty());
    }

    #[test]
    fn optimistic_drive_hide_can_be_reverted() {
        let mut state = test_drive_state(vec![("E:\\".to_string(), "ISO (E:)".to_string())]);

        assert!(state.hide_drive_optimistically("E:\\"));
        state.unhide_drive("E:\\");

        let mut scan_result = DriveScanResult {
            disks: vec![("E:\\".to_string(), "ISO (E:)".to_string())],
            cloud_roots: Vec::new(),
            unavailable_label_roots: HashSet::new(),
        };
        state.apply_optimistic_drive_filter(&mut scan_result);

        assert_eq!(
            scan_result.disks,
            vec![("E:\\".to_string(), "ISO (E:)".to_string())]
        );
    }

    #[test]
    fn unavailable_label_query_preserves_known_drive_label() {
        let state = test_drive_state(vec![("Z:\\".to_string(), "Archive (Z:)".to_string())]);
        let mut scan_result = DriveScanResult {
            disks: vec![("z:\\".to_string(), "Local Disk (Z:)".to_string())],
            cloud_roots: Vec::new(),
            unavailable_label_roots: HashSet::from(["Z:\\".to_string()]),
        };

        state.preserve_labels_from_unavailable_queries(&mut scan_result);

        assert_eq!(
            scan_result.disks,
            vec![("z:\\".to_string(), "Archive (Z:)".to_string())]
        );
    }

    #[test]
    fn successful_label_query_replaces_known_drive_label() {
        let state = test_drive_state(vec![("Z:\\".to_string(), "Old (Z:)".to_string())]);
        let mut scan_result = DriveScanResult {
            disks: vec![("Z:\\".to_string(), "New (Z:)".to_string())],
            cloud_roots: Vec::new(),
            unavailable_label_roots: HashSet::new(),
        };

        state.preserve_labels_from_unavailable_queries(&mut scan_result);

        assert_eq!(
            scan_result.disks,
            vec![("Z:\\".to_string(), "New (Z:)".to_string())]
        );
    }

    #[test]
    fn unavailable_label_for_new_drive_keeps_fallback() {
        let state = test_drive_state(Vec::new());
        let mut scan_result = DriveScanResult {
            disks: vec![("Z:\\".to_string(), "Local Disk (Z:)".to_string())],
            cloud_roots: Vec::new(),
            unavailable_label_roots: HashSet::from(["Z:\\".to_string()]),
        };

        state.preserve_labels_from_unavailable_queries(&mut scan_result);

        assert_eq!(
            scan_result.disks,
            vec![("Z:\\".to_string(), "Local Disk (Z:)".to_string())]
        );
    }
}
