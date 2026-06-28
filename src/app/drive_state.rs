use crate::domain::cloud_root::CloudRoot;
use crate::domain::file_entry::DriveInfo;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DriveScanResult {
    pub disks: Vec<(String, String)>,
    pub cloud_roots: Vec<CloudRoot>,
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
    pub drive_info_rx: Receiver<Vec<(String, DriveInfo)>>,
    pub drive_info_tx: Sender<Vec<(String, DriveInfo)>>,
    pub drive_info_cache: HashMap<String, DriveInfo>,
    pub drive_info_cache_epoch: u64,
    /// Guards concurrent background volume-info refreshes.
    pub drive_info_refresh_pending: bool,
    pub last_drive_info_refresh: Instant,
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
}
