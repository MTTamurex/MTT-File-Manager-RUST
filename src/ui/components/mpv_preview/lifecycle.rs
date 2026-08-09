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

        let event_client = match mpv.create_client(Some("mtt-preview-events")) {
            Ok(client) => Some(client),
            Err(error) => {
                log::warn!(
                    "[MpvPreview] Failed to create dedicated event client; using verified-path fallback: {:?}",
                    error
                );
                None
            }
        };

        let handle = mpv_event_loop::start_event_loop(
            mpv,
            event_client,
            mpv_event_loop::EventLoopShared::new(
                self.state.clone(),
                self.event_thread_running.clone(),
                self.tracks_need_query.clone(),
                self.file_load_gate.clone(),
                self.pending_seek.clone(),
                ctx,
            ),
        );

        self.event_thread_handle = Some(handle);
    }

    fn reset_file_state(&mut self) {
        self.cached_duration = None;
        self.cached_tracks = None;
        self.pending_external_subtitle = None;
        self.sidecar_rx = None;
        self.last_interlaced = None;
        self.last_osc_enabled = None;

        if let Ok(mut pending_seek) = self.pending_seek.write() {
            *pending_seek = None;
        }
        if let Ok(mut state) = self.state.write() {
            let volume = state.volume;
            let is_muted = state.is_muted;
            let fullscreen = state.fullscreen;
            *state = MpvState {
                volume,
                is_muted,
                fullscreen,
                ..Default::default()
            };
        }
    }

    pub(super) fn begin_file_load(&mut self) -> u64 {
        let generation = self
            .file_load_gate
            .begin_load(Self::mpv_path_string(&self.path));
        self.reset_file_state();
        generation
    }

    pub(super) fn finish_file_load(&self, generation: u64, succeeded: bool) {
        self.file_load_gate
            .finish_load_command(generation, succeeded);
    }

    pub(super) fn cancel_waiting_file_load(&self) {
        self.file_load_gate.cancel_waiting_transition();
    }

    /// Stops the selected file while retaining the MPV core, event loop and HWND.
    pub fn park_for_selection_change(&mut self) {
        let _ = self.file_load_gate.park();

        self.tracks_need_query.store(0, Ordering::Release);
        self.reset_file_state();

        if let Some(mpv) = &self.mpv {
            let _ = mpv.set_property("pause", true);
            let empty: [&str; 0] = [];
            let _ = mpv.command("stop", &empty);
        }

        self.loaded_path = None;
        self.play_on_init = false;
        self.show_player = false;
        self.file_load_gate.finish_park();
    }

    /// Points the retained preview at a new file. Loading remains lazy in `update`.
    pub fn retarget_for_playback(&mut self, path: PathBuf) {
        let already_loaded = self.path == path
            && self.loaded_path.as_ref() == Some(&path)
            && !self.file_load_gate.is_parked();

        self.show_player = true;
        if already_loaded {
            self.play_on_init = false;
            self.play();
            if let Ok(mut state) = self.state.write() {
                state.is_playing = true;
            }
            return;
        }

        if self.path != path && !self.file_load_gate.is_parked() {
            self.park_for_selection_change();
        }

        self.file_load_gate.prepare_retarget();
        self.path = path;
        self.loaded_path = None;
        self.play_on_init = true;
        self.tracks_need_query.store(0, Ordering::Release);
        self.reset_file_state();
        self.file_load_gate.finish_retarget();
    }

    /// Performs explicit MPV teardown to release decode buffers and caches immediately.
    /// This is used when closing preview/tab to avoid waiting for eventual allocator cleanup.
    pub fn shutdown(&mut self) {
        let handle = self.event_thread_handle.take();
        let has_resources = self.mpv.is_some() || handle.is_some() || self.surface.is_initialized();
        if !has_resources {
            return;
        }

        log::info!("[VIDEO] Teardown MPV preview: {}", self.path.display());
        self.file_load_gate.shutdown();
        self.tracks_need_query.store(0, Ordering::Release);
        mpv_event_loop::signal_stop(&self.event_thread_running);

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
            // Detach libmpv from the UI-owned child before destroying that HWND.
            let _ = m.set_property("wid", 0_i64);
        }

        if let Ok(mut pending_seek) = self.pending_seek.write() {
            *pending_seek = None;
        }

        self.surface.destroy();

        self.cached_duration = None;
        self.cached_tracks = None;
        self.loaded_path = None;
        self.last_profile_was_docked = None;
        self.show_player = false;
        self.is_visible = false;
        self.surface.reset_rect();
        let mpv = self.mpv.take();
        spawn_mpv_reaper(handle, mpv);
    }
}

fn join_then_drop<T>(handle: Option<thread::JoinHandle<()>>, resource: Option<T>) {
    if let Some(handle) = handle {
        if handle.join().is_err() {
            log::warn!("[MpvPreview] Event loop thread panicked");
        }
    }
    drop(resource);
}

fn spawn_mpv_reaper(handle: Option<thread::JoinHandle<()>>, mpv: Option<Arc<mpv::Mpv>>) {
    if handle.is_none() && mpv.is_none() {
        return;
    }

    let payload = Arc::new(std::sync::Mutex::new(Some((handle, mpv))));
    let worker_payload = payload.clone();
    let spawn_result = thread::Builder::new()
        .name("mpv-preview-reaper".into())
        .spawn(move || {
            let resources = worker_payload
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some((handle, mpv)) = resources {
                join_then_drop(handle, mpv);
            }
        });

    if let Err(error) = spawn_result {
        log::error!("[MpvPreview] Failed to spawn MPV reaper: {error}");
        // Resource exhaustion must not move libmpv destruction back to the UI.
        // Leak only on this exceptional path; process teardown will reclaim it.
        std::mem::forget(payload);
    }
}

#[cfg(test)]
mod tests {
    use super::join_then_drop;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct DropProbe {
        worker_finished: Arc<AtomicBool>,
        dropped_after_join: Arc<AtomicBool>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.dropped_after_join.store(
                self.worker_finished.load(Ordering::Acquire),
                Ordering::Release,
            );
        }
    }

    #[test]
    fn reaper_joins_before_dropping_resource() {
        let worker_finished = Arc::new(AtomicBool::new(false));
        let dropped_after_join = Arc::new(AtomicBool::new(false));
        let worker_flag = worker_finished.clone();
        let handle = std::thread::spawn(move || {
            worker_flag.store(true, Ordering::Release);
        });
        let probe = DropProbe {
            worker_finished,
            dropped_after_join: dropped_after_join.clone(),
        };

        join_then_drop(Some(handle), Some(probe));

        assert!(dropped_after_join.load(Ordering::Acquire));
    }
}
