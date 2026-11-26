//! Which GPU mpv renders on, and whether it can run NVIDIA RTX Video Super
//! Resolution / RTX HDR (`d3d11vpp` filters that run on mpv's own D3D11 device).
//!
//! mpv follows the GPU the app renders on (the GPU preference in settings),
//! even when the monitor is connected to another GPU.

use crate::infrastructure::windows::gpu_topology;

/// Passes the app's rendering GPU to the standalone player process.
pub const VIDEO_GPU_ENV: &str = "MTT_VIDEO_GPU_ADAPTER";

pub struct VideoGpu {
    /// Value for mpv's `d3d11-adapter`; `None` keeps D3D11's default adapter.
    d3d11_adapter: Option<String>,
    rtx_supported: bool,
}

impl VideoGpu {
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
