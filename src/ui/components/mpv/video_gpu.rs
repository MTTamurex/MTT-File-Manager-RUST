//! Which GPU mpv decodes and renders video on, and whether it can run NVIDIA
//! RTX Video Super Resolution / RTX HDR (`d3d11vpp` filters that run on mpv's
//! own D3D11 device).

use std::sync::atomic::{AtomicU8, Ordering};

use crate::infrastructure::windows::gpu_topology;

pub const PREFERENCE_KEY: &str = "video_gpu_preference";

/// Passes the resolved video GPU name to the standalone player process.
pub const VIDEO_GPU_ENV: &str = "MTT_VIDEO_GPU_ADAPTER";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoGpuPreference {
    HighPerformance,
    LowPower,
    /// The GPU driving the monitor, so video frames are never copied between GPUs.
    #[default]
    Automatic,
}

impl VideoGpuPreference {
    pub const ALL: [Self; 3] = [Self::HighPerformance, Self::LowPower, Self::Automatic];

    /// Unknown values, including the removed `follow_app`, fall back to
    /// Automatic.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("high_performance") => Self::HighPerformance,
            Some("low_power") => Self::LowPower,
            _ => Self::Automatic,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::HighPerformance => "high_performance",
            Self::LowPower => "low_power",
            Self::Automatic => "automatic",
        }
    }

    /// Process-wide copy of the setting, read when a player creates mpv.
    pub fn current() -> Self {
        Self::ALL[CURRENT.load(Ordering::Relaxed) as usize]
    }

    pub fn set_current(self) {
        // Variants are declared in `ALL` order, so the discriminant is the index.
        CURRENT.store(self as u8, Ordering::Relaxed);
    }

    /// Name for mpv's `d3d11-adapter`; `None` keeps D3D11's default adapter.
    pub fn adapter_name(self) -> Option<String> {
        let adapter = match self {
            Self::HighPerformance => gpu_topology::adapter_by_power_preference(true),
            Self::LowPower => gpu_topology::adapter_by_power_preference(false),
            Self::Automatic => gpu_topology::primary_display_adapter(),
        };
        adapter.map(|adapter| adapter.name)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(VideoGpuPreference::Automatic as u8);

pub struct VideoGpu {
    /// Value for mpv's `d3d11-adapter`; `None` keeps D3D11's default adapter.
    d3d11_adapter: Option<String>,
    rtx_supported: bool,
}

impl VideoGpu {
    /// The GPU chosen in settings, for the in-process (docked) player.
    pub fn from_settings() -> Self {
        Self::resolve(VideoGpuPreference::current().adapter_name().as_deref())
    }

    /// Enumerates DXGI adapters, so call it before creating mpv: done between
    /// the VO setup and `loadfile`, the delay lets mpv open its idle window.
    pub fn resolve(d3d11_adapter: Option<&str>) -> Self {
        let d3d11_adapter = d3d11_adapter
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        let adapter = gpu_topology::mpv_d3d11_adapter(d3d11_adapter.as_deref());
        let rtx_supported = adapter.as_ref().is_some_and(|a| a.is_nvidia_rtx());
        log::info!(
            "[VIDEO-GPU] requested={} resolved={} rtx_vsr_hdr={}",
            d3d11_adapter.as_deref().unwrap_or("default"),
            adapter.as_ref().map_or("unknown", |a| a.name.as_str()),
            rtx_supported
        );
        Self {
            d3d11_adapter,
            rtx_supported,
        }
    }

    /// Sets `d3d11-adapter` and publishes `user-data/vsr/rtx-supported` for
    /// vsr.lua and the OSC. Call before `loadfile` so the VO opens on this GPU.
    pub fn apply(&self, mpv: &mpv::Mpv) {
        if let Some(name) = &self.d3d11_adapter {
            // mpv matches this against the start of the DXGI adapter description.
            let _ = mpv.set_property("d3d11-adapter", name.as_str());
        }
        let _ = mpv.set_property("user-data/vsr/rtx-supported", self.rtx_supported);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_defaults_to_the_monitor_gpu_and_round_trips() {
        assert_eq!(
            VideoGpuPreference::parse(None),
            VideoGpuPreference::Automatic
        );
        assert_eq!(
            VideoGpuPreference::parse(Some("invalid")),
            VideoGpuPreference::Automatic
        );
        assert_eq!(
            VideoGpuPreference::parse(Some("follow_app")),
            VideoGpuPreference::Automatic
        );
        for preference in VideoGpuPreference::ALL {
            assert_eq!(
                VideoGpuPreference::parse(Some(preference.key())),
                preference
            );
        }
    }
}
