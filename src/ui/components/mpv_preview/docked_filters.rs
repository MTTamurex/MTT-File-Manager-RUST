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

    /// Switches between the low-memory detail-panel profile and the legacy
    /// in-process detached profile without adding software video filters.
    pub(super) fn update_docked_profile(&mut self) {
        let should_use_docked_profile = self.is_docked();
        if self.last_profile_was_docked == Some(should_use_docked_profile) {
            return;
        }

        let Some(m) = &self.mpv else {
            return;
        };

        if should_use_docked_profile {
            let _ = m.set_property("video-sync", "audio");
            let _ = m.set_property("interpolation", false);
            let _ = m.set_property("tscale", "linear");
            let _ = m.set_property("cache", "no");
            let _ = m.set_property("demuxer-max-bytes", MPV_DOCKED_DEMUXER_MAX_BYTES);
            let _ = m.set_property("demuxer-max-back-bytes", MPV_DOCKED_DEMUXER_MAX_BACK_BYTES);

            log::info!(
                "[MpvPreview] Applied docked low-memory profile: cache=no, demux={}MB/{}MB, vf='{}'",
                MPV_DOCKED_DEMUXER_MAX_BYTES / (1024 * 1024),
                MPV_DOCKED_DEMUXER_MAX_BACK_BYTES / (1024 * 1024),
                m.get_property::<String>("vf").unwrap_or_default()
            );
        } else {
            let _ = m.set_property("cache", "yes");
            let _ = m.set_property("cache-secs", MPV_DETACHED_CACHE_SECS);
            let _ = m.set_property("demuxer-readahead-secs", MPV_DETACHED_READAHEAD_SECS);
            let _ = m.set_property("demuxer-max-bytes", MPV_DETACHED_DEMUXER_MAX_BYTES);
            let _ = m.set_property(
                "demuxer-max-back-bytes",
                MPV_DETACHED_DEMUXER_MAX_BACK_BYTES,
            );
        }

        self.last_profile_was_docked = Some(should_use_docked_profile);
        self.configure_media_graph();
    }

    pub(super) fn configure_media_graph(&self) {
        let Some(m) = &self.mpv else {
            return;
        };

        let is_audio = self
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(crate::infrastructure::windows::is_audio_extension)
            .unwrap_or(false);
        let graph = if !is_audio {
            ""
        } else if self.is_docked() {
            MPV_DOCKED_AUDIO_VISUALIZATION
        } else {
            MPV_DETACHED_AUDIO_VISUALIZATION
        };

        let _ = m.set_property("force-window", false);
        let _ = m.set_property("lavfi-complex", graph);
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
        if interlaced {
            let _ = m.set_property("deinterlace", "yes");
        } else {
            let _ = m.set_property("deinterlace", "no");
        }
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
            Ok(())
        } else {
            Err("MPV instance not initialized".to_string())
        }
    }
}
