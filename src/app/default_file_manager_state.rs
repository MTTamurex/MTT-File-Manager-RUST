use std::sync::mpsc::Receiver;
use std::time::Instant;

use crate::infrastructure::windows::default_file_manager::{
    RegistrationStatus, RegistryActionError,
};

pub struct DefaultFileManagerSettingsState {
    pub status: RegistrationStatus,
    pub(crate) last_status_refresh: Instant,
    pub(crate) result_receiver: Option<Receiver<Result<RegistrationStatus, RegistryActionError>>>,
}

impl DefaultFileManagerSettingsState {
    pub fn new() -> Self {
        Self {
            status: crate::infrastructure::windows::default_file_manager::registration_status(),
            last_status_refresh: Instant::now(),
            result_receiver: None,
        }
    }
}

impl Default for DefaultFileManagerSettingsState {
    fn default() -> Self {
        Self::new()
    }
}
