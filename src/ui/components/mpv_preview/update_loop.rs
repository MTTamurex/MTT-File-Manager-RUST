use super::*;

impl MpvPreview {
    pub fn update(&mut self, _ui: &mut egui::Ui, _frame: Option<&eframe::Frame>) {
        if !self.show_player {
            self.osc_active = false;
            self.set_visibility(false);
            return;
        }

        let ui = _ui;

        // Reserve space for the video. If forced_size is set (detached mode with control bar), use it.
        let size = if let Some(forced) = self.forced_size {
            forced
        } else if self.is_detached() {
            ui.available_size()
        } else {
            let available = ui.available_size();
            let preview_height = (available.x * 0.6).min(300.0);
            egui::vec2(available.x, preview_height)
        };
        let (rect, _response) = ui.allocate_exact_size(size, egui::Sense::hover());

        // Track mouse activity for autohide controls (movement-based)
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pos) {
                let moved = self
                    .last_mouse_pos
                    .map(|prev| prev.distance(pos) > 2.0)
                    .unwrap_or(true);
                if moved {
                    self.last_mouse_activity = Some(Instant::now());
                    self.last_mouse_pos = Some(pos);
                }
            }
        }

        // Init MPV and child window
        if self.mpv.is_none() {
            match Self::create_mpv_instance() {
                Ok(m) => {
                    let m = Arc::new(m);
                    let _ = m.set_property("keep-open", "yes");

                    // Mandatory configuration for NVIDIA RTX VSR
                    // We must use D3D11 backend and D3D11 VA hardware decoding
                    if let Err(e) = m.set_property("vo", "gpu") {
                        log::warn!("[MpvPreview] Failed to set vo=gpu: {:?}", e);
                    }
                    if let Err(e) = m.set_property("gpu-api", "d3d11") {
                        log::warn!("[MpvPreview] Failed to set gpu-api=d3d11: {:?}", e);
                    }
                    if let Err(e) = m.set_property("gpu-context", "d3d11") {
                        log::warn!("[MpvPreview] Failed to set gpu-context=d3d11: {:?}", e);
                    }
                    if let Err(e) = m.set_property("hwdec", "d3d11va") {
                        log::warn!("[MpvPreview] Failed to set hwdec=d3d11va: {:?}", e);
                    }

                    // Use a balanced baseline profile for 4K stability.
                    // display-resample + interpolation can overload some GPUs in fullscreen.
                    let _ = m.set_property("video-sync", "audio");
                    let _ = m.set_property("interpolation", false);
                    let _ = m.set_property("tscale", "linear");
                    let _ = m.set_property("framedrop", "vo");

                    // Bound demux/cache memory so high-bitrate files do not balloon RAM usage.
                    let _ = m.set_property("cache", "yes");
                    let _ = m.set_property("cache-secs", MPV_DEFAULT_CACHE_SECS);
                    let _ = m.set_property("demuxer-readahead-secs", MPV_DEFAULT_READAHEAD_SECS);
                    let _ = m.set_property("demuxer-max-bytes", MPV_DEFAULT_DEMUXER_MAX_BYTES);
                    let _ = m
                        .set_property("demuxer-max-back-bytes", MPV_DEFAULT_DEMUXER_MAX_BACK_BYTES);

                    let _ = m.set_property("pause", true);

                    // PERF FASE 2: Start async event loop for push-based state updates
                    self.start_event_loop_internal(m.clone(), ui.ctx().clone());

                    self.mpv = Some(m);
                    self.set_audio_normalizer(self.audio_normalizer_enabled);

                    // Apply initial volume
                    self.set_volume(self.initial_volume);

                    if MPV_OSC_POC_ENABLED {
                        if let Some(mpv_ref) = &self.mpv {
                            let input_cursor = mpv_ref.get_property::<bool>("input-cursor").ok();
                            let script_count = mpv_ref.get_property::<i64>("script-list/count").ok();
                            log::debug!(
                                "[MpvPreview][OSC-POC] input-cursor={:?}, script-list/count={:?}",
                                input_cursor, script_count
                            );
                        }
                    }
                }
                Err(e) => {
                    log::error!("[MpvPreview] Failed to create MPV: {:?}", e);
                    return;
                }
            }
        }

        self.surface.ensure_main_hwnd(_frame);
        if let Some(m) = &self.mpv {
            self.surface.ensure_child_window(m);
        }

        if let Some(m) = self.mpv.clone() {
            self.sync_fullscreen_from_mpv(ui, &m);
            self.sync_osc_runtime_state(&m);
        }

        // Load file once
        if self.loaded_path.as_ref() != Some(&self.path) {
            if let Some(m) = &self.mpv {
                let path_str = self.path.to_string_lossy().to_string();
                let _ = m.command("loadfile", &[&path_str]);

                // Prefer sidecar subtitle when available (movie.srt, movie.en.srt, etc.)
                self.pending_external_subtitle = mpv_playback::find_sidecar_subtitle(&self.path);

                if self.play_on_init {
                    let _ = m.set_property("pause", false);
                    self.play_on_init = false;
                }
            }
            self.loaded_path = Some(self.path.clone());

            // Clear cached values for new file
            self.cached_duration = None;
            self.cached_tracks = None;

            // Defensive cleanup: ensure docked-only filters are not carried across files.
            self.update_docked_downscale(false);
        }

        // Apply docked-mode downscale + FPS limit (dynamic, reversible, no player restart)
        let is_detached = self.is_detached();
        if is_detached == self.docked_downscale_applied
            || is_detached == self.docked_fps_limit_applied
        {
            self.update_docked_downscale(false);
        }

        // PERF FASE 2: State updates now handled by async event loop (zero polling overhead)
        // Only tracks still need manual fetching (heavy JSON parse, done once per file)
        if let Some(m) = self.mpv.clone() {
            let file_ready = mpv_playback::is_file_ready(&m);
            if file_ready {
                if let Some(sidecar) = self.pending_external_subtitle.take() {
                    if let Err(e) = self.load_external_subtitle(&sidecar) {
                        log::error!("[MPV] Failed to auto-load sidecar subtitle: {}", e);
                    }
                }
            }

            if self.cached_tracks.is_none() && file_ready {
                let (audio_tracks, sub_tracks): (Vec<TrackInfo>, Vec<TrackInfo>) =
                    mpv_playback::query_tracks(&m);

                self.cached_tracks = Some((audio_tracks.clone(), sub_tracks.clone()));

                if let Ok(mut state) = self.state.write() {
                    state.audio_tracks = audio_tracks;
                    state.subtitle_tracks = sub_tracks;
                }
            } else if let Some((ref audio, ref subs)) = self.cached_tracks {
                if let Ok(mut state) = self.state.write() {
                    state.audio_tracks = audio.clone();
                    state.subtitle_tracks = subs.clone();
                }
            }
        }

        if self.last_deinterlace_check.elapsed() >= Duration::from_millis(500) {
            self.update_deinterlace_filter();
            self.last_deinterlace_check = Instant::now();
        }

        if self.osc_active {
            if let Some(m) = self.mpv.clone() {
                self.forward_osc_input(ui, rect, &m);
            }
        }

        self.surface.sync_rect(ui, rect);

        // Keep MPV focus while native OSC is active so it can handle input events.
        let should_force_main_focus = !self.osc_active;
        if should_force_main_focus {
            self.surface.ensure_focus_on_main();
        }

        // Context menu removed - controls now in control bar.
        self.set_visibility(self.is_visible);
    }
}
