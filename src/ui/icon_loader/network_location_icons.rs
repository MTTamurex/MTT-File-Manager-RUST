//! A shared, asynchronously loaded native icon for saved network locations.

use super::*;

const NETWORK_LOCATION_ICON_KEY: &str = "network-location-folder";

impl IconLoader {
    pub fn get_or_load_network_location_icon(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<egui::TextureHandle> {
        if let Some(icon) = self.drive_icon_cache.get(NETWORK_LOCATION_ICON_KEY) {
            return Some(icon.clone());
        }
        if self
            .failed_drive_icons
            .peek(NETWORK_LOCATION_ICON_KEY)
            .is_some()
            || self.loading_drive_icons.contains(NETWORK_LOCATION_ICON_KEY)
        {
            return None;
        }
        let active = self.auxiliary_icon_threads.clone();
        if !super::try_reserve_auxiliary_icon_thread(&active) {
            return None;
        }
        self.loading_drive_icons
            .insert(NETWORK_LOCATION_ICON_KEY.into());
        let tx = self.icon_result_tx.clone();
        let worker_active = active.clone();
        let ctx = ctx.clone();
        let result = std::thread::Builder::new()
            .name("network-location-icon-worker".into())
            .spawn(move || {
                let _guard = super::ThreadCountGuard(worker_active);
                let _com = super::ComStaGuard::new();
                let data = windows::extract_network_location_icon(IconSize::Jumbo).ok();
                let _ = tx.send(AsyncIconResult {
                    key: NETWORK_LOCATION_ICON_KEY.into(),
                    data,
                });
                ctx.request_repaint();
            });
        if let Err(error) = result {
            active.fetch_sub(1, Ordering::Relaxed);
            self.loading_drive_icons.remove(NETWORK_LOCATION_ICON_KEY);
            log::error!("[Icon] Failed to spawn network-location-icon-worker: {error}");
        }
        None
    }
}
