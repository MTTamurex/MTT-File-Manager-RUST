use super::*;

impl MpvPreview {
    fn script_vsr_enabled(mpv: &mpv::Mpv) -> Option<bool> {
        mpv.get_property::<bool>("user-data/vsr/vsr-enabled").ok()
    }

    fn script_hdr_enabled(mpv: &mpv::Mpv) -> Option<bool> {
        mpv.get_property::<bool>("user-data/vsr/hdr-enabled").ok()
    }

    fn script_rtx_supported(mpv: &mpv::Mpv) -> Option<bool> {
        mpv.get_property::<bool>("user-data/vsr/rtx-supported").ok()
    }

    fn remove_legacy_direct_vsr_filter(mpv: &mpv::Mpv) {
        let Ok(current_vf) = mpv.get_property::<String>("vf") else {
            return;
        };
        let cleaned_vf =
            mpv_filters::remove_vf_filter(&current_vf, mpv_filters::LEGACY_DIRECT_VSR_MARKER);
        if cleaned_vf != current_vf {
            let _ = mpv.set_property("vf", cleaned_vf);
        }
    }

    fn log_vsr_pipeline(mpv: &mpv::Mpv, context: &str) {
        let vf = mpv.get_property::<String>("vf").unwrap_or_default();
        let vo = mpv.get_property::<String>("vo").unwrap_or_default();
        let gpu_api = mpv.get_property::<String>("gpu-api").unwrap_or_default();
        let hwdec = mpv
            .get_property::<String>("hwdec-current")
            .unwrap_or_default();
        let src_w = mpv.get_property::<i64>("video-params/w").unwrap_or(0);
        let src_h = mpv.get_property::<i64>("video-params/h").unwrap_or(0);
        let out_w = mpv.get_property::<i64>("video-out-params/w").unwrap_or(0);
        let out_h = mpv.get_property::<i64>("video-out-params/h").unwrap_or(0);
        let script_vsr = Self::script_vsr_enabled(mpv);
        let script_hdr = Self::script_hdr_enabled(mpv);
        let rtx_supported = Self::script_rtx_supported(mpv);

        log::info!(
            "[MpvPreview] VSR pipeline {}: vf='{}', vo='{}', gpu-api='{}', hwdec-current='{}', src={}x{}, out={}x{}, script_vsr={:?}, script_hdr={:?}, rtx_supported={:?}",
            context,
            vf,
            vo,
            gpu_api,
            hwdec,
            src_w,
            src_h,
            out_w,
            out_h,
            script_vsr,
            script_hdr,
            rtx_supported
        );
    }

    pub(super) fn sync_vsr_flags_from_mpv(&mut self, mpv: &mpv::Mpv) {
        if let Some(enabled) = Self::script_vsr_enabled(mpv) {
            self.is_vsr_enabled = enabled;
        }
        if let Some(supported) = Self::script_rtx_supported(mpv) {
            self.is_rtx_supported = supported;
        }
    }

    /// Applies or removes docked-mode downscale and FPS limiting without restarting playback.
    /// `force_reapply` is used when external changes (e.g., VSR) replace the filter chain.
    pub(super) fn update_docked_downscale(&mut self, force_reapply: bool) {
        let should_limit = self.is_docked();
        let Some(m) = &self.mpv else {
            return;
        };

        let current_vf = m.get_property::<String>("vf").unwrap_or_default();
        let has_downscale = current_vf.contains(mpv_filters::DOCKED_DOWNSCALE_MARKER);
        let has_fps_limit = current_vf.contains(mpv_filters::DOCKED_FPS_MARKER);

        if should_limit {
            if force_reapply || !has_downscale || !has_fps_limit {
                if self.docked_prev_vf.is_none() {
                    self.docked_prev_vf = Some(current_vf.clone());
                }

                let mut new_vf = current_vf.clone();
                if !has_downscale {
                    new_vf = if new_vf.trim().is_empty() {
                        mpv_filters::DOCKED_DOWNSCALE_FILTER.to_string()
                    } else {
                        format!("{},{}", new_vf, mpv_filters::DOCKED_DOWNSCALE_FILTER)
                    };
                }
                if !has_fps_limit {
                    new_vf = if new_vf.trim().is_empty() {
                        mpv_filters::DOCKED_FPS_FILTER.to_string()
                    } else {
                        format!("{},{}", new_vf, mpv_filters::DOCKED_FPS_FILTER)
                    };
                }
                let _ = m.set_property("vf", new_vf);
            }

            if self.docked_prev_video_sync.is_none() {
                self.docked_prev_video_sync = m.get_property::<String>("video-sync").ok();
            }
            if self.docked_prev_interpolation.is_none() {
                self.docked_prev_interpolation = m.get_property::<bool>("interpolation").ok();
            }
            if self.docked_prev_tscale.is_none() {
                self.docked_prev_tscale = m.get_property::<String>("tscale").ok();
            }

            let _ = m.set_property("video-sync", "audio");
            let _ = m.set_property("interpolation", false);
            let _ = m.set_property("tscale", "linear");

            if self.docked_prev_cache.is_none() {
                self.docked_prev_cache = m.get_property::<String>("cache").ok();
            }
            if self.docked_prev_cache_secs.is_none() {
                self.docked_prev_cache_secs = m.get_property::<f64>("cache-secs").ok();
            }
            if self.docked_prev_readahead_secs.is_none() {
                self.docked_prev_readahead_secs =
                    m.get_property::<f64>("demuxer-readahead-secs").ok();
            }
            if self.docked_prev_demuxer_max_bytes.is_none() {
                self.docked_prev_demuxer_max_bytes =
                    m.get_property::<i64>("demuxer-max-bytes").ok();
            }
            if self.docked_prev_demuxer_max_back_bytes.is_none() {
                self.docked_prev_demuxer_max_back_bytes =
                    m.get_property::<i64>("demuxer-max-back-bytes").ok();
            }

            let _ = m.set_property("cache", "yes");
            let _ = m.set_property("cache-secs", MPV_DOCKED_CACHE_SECS);
            let _ = m.set_property("demuxer-readahead-secs", MPV_DOCKED_READAHEAD_SECS);
            let _ = m.set_property("demuxer-max-bytes", MPV_DOCKED_DEMUXER_MAX_BYTES);
            let _ = m.set_property("demuxer-max-back-bytes", MPV_DOCKED_DEMUXER_MAX_BACK_BYTES);

            self.docked_downscale_applied = true;
            self.docked_fps_limit_applied = true;
        } else if self.docked_downscale_applied
            || self.docked_fps_limit_applied
            || has_downscale
            || has_fps_limit
        {
            // Robust detach cleanup: ensure docked-only filters are removed even if
            // internal flags drift from the actual vf chain.
            let cleaned_vf = mpv_filters::remove_vf_filter(
                &mpv_filters::remove_vf_filter(&current_vf, mpv_filters::DOCKED_DOWNSCALE_MARKER),
                mpv_filters::DOCKED_FPS_MARKER,
            );
            let restore_vf = if has_downscale || has_fps_limit {
                cleaned_vf
            } else {
                self.docked_prev_vf
                    .clone()
                    .unwrap_or_else(|| current_vf.clone())
            };
            let _ = m.set_property("vf", restore_vf);
            self.docked_prev_vf = None;

            if let Some(prev) = self.docked_prev_video_sync.take() {
                let _ = m.set_property("video-sync", prev);
            }
            if let Some(prev) = self.docked_prev_interpolation.take() {
                let _ = m.set_property("interpolation", prev);
            }
            if let Some(prev) = self.docked_prev_tscale.take() {
                let _ = m.set_property("tscale", prev);
            }

            self.docked_prev_cache = None;
            self.docked_prev_cache_secs = None;
            self.docked_prev_readahead_secs = None;
            self.docked_prev_demuxer_max_bytes = None;
            self.docked_prev_demuxer_max_back_bytes = None;

            let _ = m.set_property("cache", "yes");
            let _ = m.set_property("cache-secs", MPV_DETACHED_CACHE_SECS);
            let _ = m.set_property("demuxer-readahead-secs", MPV_DETACHED_READAHEAD_SECS);
            let _ = m.set_property("demuxer-max-bytes", MPV_DETACHED_DEMUXER_MAX_BYTES);
            let _ = m.set_property(
                "demuxer-max-back-bytes",
                MPV_DETACHED_DEMUXER_MAX_BACK_BYTES,
            );

            self.docked_downscale_applied = false;
            self.docked_fps_limit_applied = false;
        }
    }

    /// Apply deinterlace filter based on pre-detected interlaced state.
    /// Detection is now handled by the background event loop.
    pub(super) fn apply_deinterlace_state(&mut self, interlaced: Option<bool>) {
        let Some(m) = &self.mpv else {
            return;
        };
        let interlaced = match interlaced {
            Some(value) => value,
            None => {
                let _ = m.set_property("deinterlace", "auto");
                return;
            }
        };
        let current_vf = m.get_property::<String>("vf").unwrap_or_default();
        let has_deinterlace = current_vf.contains(mpv_filters::DEINTERLACE_MARKER);

        if interlaced && !has_deinterlace {
            let _ = m.set_property("deinterlace", "yes");
            let new_vf =
                mpv_filters::append_vf_filter(&current_vf, mpv_filters::DEINTERLACE_FILTER);
            let _ = m.set_property("vf", new_vf);
            self.update_prev_vf_deinterlace(true);
        } else if !interlaced && has_deinterlace {
            let _ = m.set_property("deinterlace", "no");
            let new_vf =
                mpv_filters::remove_vf_filter(&current_vf, mpv_filters::DEINTERLACE_MARKER);
            let _ = m.set_property("vf", new_vf);
            self.update_prev_vf_deinterlace(false);
        } else if !interlaced {
            let _ = m.set_property("deinterlace", "no");
        }
    }

    pub(super) fn update_prev_vf_deinterlace(&mut self, apply: bool) {
        let Some(prev) = self.docked_prev_vf.clone() else {
            return;
        };
        let updated = if apply {
            if prev.contains(mpv_filters::DEINTERLACE_MARKER) {
                prev
            } else {
                mpv_filters::append_vf_filter(&prev, mpv_filters::DEINTERLACE_FILTER)
            }
        } else if prev.contains(mpv_filters::DEINTERLACE_MARKER) {
            mpv_filters::remove_vf_filter(&prev, mpv_filters::DEINTERLACE_MARKER)
        } else {
            prev
        };
        self.docked_prev_vf = Some(updated);
    }

    /// Enables NVIDIA RTX Video Super Resolution (VSR).
    ///
    /// Requires MPV to be initialized with:
    /// - vo=gpu
    /// - gpu-api=d3d11
    /// - hwdec=d3d11va
    pub fn enable_nvidia_vsr(&mut self) -> Result<(), String> {
        if let Some(m) = &self.mpv {
            Self::remove_legacy_direct_vsr_filter(m);

            if let Some(script_enabled) = Self::script_vsr_enabled(m) {
                if Self::script_rtx_supported(m) == Some(false) {
                    return Err("NVIDIA RTX GPU not detected".to_string());
                }

                if !script_enabled {
                    m.command("script-message", &["toggle-vsr"])
                        .map_err(|e| format!("Failed to enable VSR via script: {:?}", e))?;
                }
            } else {
                let current_vf = m.get_property::<String>("vf").unwrap_or_default();
                let new_vf = mpv_filters::append_vf_filter(
                    &current_vf,
                    mpv_filters::LEGACY_DIRECT_VSR_MARKER,
                );
                m.set_property("vf", new_vf)
                    .map_err(|e| format!("Failed to enable VSR fallback: {:?}", e))?;
            }

            self.is_vsr_enabled = true;
            log::info!("[MpvPreview] NVIDIA VSR Enabled");
            Self::log_vsr_pipeline(m, "enable_requested");
            self.update_docked_downscale(true);
            Ok(())
        } else {
            Err("MPV instance not initialized".to_string())
        }
    }

    /// Disables VSR by clearing the video filter chain.
    pub fn disable_vsr(&mut self) -> Result<(), String> {
        if let Some(m) = &self.mpv {
            Self::remove_legacy_direct_vsr_filter(m);

            if let Some(script_enabled) = Self::script_vsr_enabled(m) {
                if script_enabled {
                    m.command("script-message", &["toggle-vsr"])
                        .map_err(|e| format!("Failed to disable VSR via script: {:?}", e))?;
                }
            }

            self.is_vsr_enabled = false;
            log::info!("[MpvPreview] VSR Disabled");
            Self::log_vsr_pipeline(m, "disable_requested");
            self.update_docked_downscale(true);
            Ok(())
        } else {
            Err("MPV instance not initialized".to_string())
        }
    }
}
