use crate::domain::file_entry::DriveInfo;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

pub struct DriveState {
    pub disks: Vec<(String, String)>,
    pub last_drive_refresh: Instant,
    pub last_drive_bitmask: u32,
    pub drive_scan_pending: bool,
    pub drive_scan_rx: Receiver<Vec<(String, String)>>,
    pub drive_scan_tx: Sender<Vec<(String, String)>>,
    pub drive_info_rx: Receiver<Vec<(String, DriveInfo)>>,
    pub drive_info_tx: Sender<Vec<(String, DriveInfo)>>,
    pub drive_info_cache: HashMap<String, DriveInfo>,
    /// Guards concurrent background volume-info refreshes.
    pub drive_info_refresh_pending: bool,
    pub last_drive_info_refresh: Instant,
}
