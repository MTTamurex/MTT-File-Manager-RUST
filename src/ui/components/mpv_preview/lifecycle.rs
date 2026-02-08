use super::*;

impl MpvPreview {
    /// PERF FASE 2: Starts async polling thread for offloading FFI calls from main thread
    ///
    /// This moves the polling to a background thread, preventing main thread blocking.
    /// Polls at 4 FPS (250ms) but from a separate thread, keeping UI responsive.
    pub(super) fn start_event_loop_internal(&mut self, mpv: Arc<mpv::Mpv>, ctx: egui::Context) {
        if self.event_thread_running.load(Ordering::Relaxed) {
            return;
        }

        let handle = mpv_event_loop::start_event_loop(
            mpv,
            self.state.clone(),
            self.event_thread_running.clone(),
            ctx,
        );

        self.event_thread_handle = Some(handle);
    }

    /// Performs explicit MPV teardown to release decode buffers and caches immediately.
    /// This is used when closing preview/tab to avoid waiting for eventual allocator cleanup.
    pub fn shutdown(&mut self) {
        eprintln!("[VIDEO] Teardown MPV preview: {}", self.path.display());

        if let Some(m) = &self.mpv {
            let _ = m.set_property("pause", true);
            let _ = m.set_property("keep-open", "no");
            let _ = m.set_property("cache", "no");
            let _ = m.set_property("vid", "no");
            let _ = m.set_property("aid", "no");
            let _ = m.set_property("sid", "no");
            let empty: [&str; 0] = [];
            let _ = m.command("stop", &empty);
            let _ = m.command("playlist-clear", &empty);
        }

        mpv_event_loop::stop_event_loop(
            self.event_thread_running.clone(),
            self.event_thread_handle.take(),
        );

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.mpv_hwnd.take() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                let _ = DestroyWindow(hwnd);
            }
        }

        self.cached_duration = None;
        self.cached_tracks = None;
        self.loaded_path = None;
        self.show_player = false;
        self.is_visible = false;
        self.last_rect = egui::Rect::NAN;
        self.mpv = None;
    }
}
