use std::sync::mpsc::{self, TryRecvError};

use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::default_file_manager::{self, RegistryActionError};

#[derive(Clone, Copy)]
enum RegistryOperation {
    FolderDefault(bool),
    WinE(bool),
}

impl ImageViewerApp {
    pub fn start_default_file_manager_action(&mut self, enable: bool) {
        self.start_default_file_manager_registry_action(RegistryOperation::FolderDefault(enable));
    }

    pub fn start_win_e_action(&mut self, enable: bool) {
        self.start_default_file_manager_registry_action(RegistryOperation::WinE(enable));
    }

    fn start_default_file_manager_registry_action(&mut self, action: RegistryOperation) {
        if self.default_file_manager_settings.result_receiver.is_some() {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        self.default_file_manager_settings.result_receiver = Some(receiver);
        let owner_window = self.native_hwnd.map(|hwnd| hwnd.0 as isize);
        let context_menu_label = rust_i18n::t!("context_menu.open_in_mtt").to_string();
        let ctx = self.ui_ctx.clone();
        let worker = std::thread::Builder::new()
            .name("default-file-manager-registry".to_string())
            .spawn(move || {
                let result = match action {
                    RegistryOperation::FolderDefault(true) => {
                        default_file_manager::set_as_default(owner_window, &context_menu_label)
                    }
                    RegistryOperation::FolderDefault(false) => {
                        default_file_manager::restore_explorer(owner_window, &context_menu_label)
                    }
                    RegistryOperation::WinE(enable) => {
                        default_file_manager::set_win_e_enabled(enable, owner_window)
                    }
                };
                let _ = sender.send(result);
                ctx.request_repaint();
            });

        if worker.is_err() {
            self.default_file_manager_settings.result_receiver = None;
            self.default_file_manager_settings.last_status_refresh = std::time::Instant::now();
            log::error!("[DEFAULT-FILE-MANAGER] Failed to start registry operation worker");
        }
    }

    pub fn poll_default_file_manager_action(&mut self) {
        let operation_pending = self.default_file_manager_settings.result_receiver.is_some();
        let result = self
            .default_file_manager_settings
            .result_receiver
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(RegistryActionError::Failed)),
            });

        let Some(result) = result else {
            if !operation_pending
                && self.navigation_state.show_settings_window
                && self
                    .default_file_manager_settings
                    .last_status_refresh
                    .elapsed()
                    >= std::time::Duration::from_secs(1)
            {
                self.default_file_manager_settings.status =
                    default_file_manager::registration_status();
                self.default_file_manager_settings.last_status_refresh = std::time::Instant::now();
            }
            return;
        };

        self.default_file_manager_settings.result_receiver = None;
        self.default_file_manager_settings.last_status_refresh = std::time::Instant::now();
        match result {
            Ok(status) => {
                self.default_file_manager_settings.status = status;
            }
            Err(RegistryActionError::Cancelled) => {
                self.default_file_manager_settings.status =
                    default_file_manager::registration_status();
            }
            Err(RegistryActionError::Failed) => {
                self.default_file_manager_settings.status =
                    default_file_manager::registration_status();
                log::error!("[DEFAULT-FILE-MANAGER] Registry operation failed");
            }
        }
    }
}
