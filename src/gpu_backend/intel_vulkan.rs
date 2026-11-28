//! Renders on Vulkan instead of DirectX 12 when the render GPU is Intel.
//!
//! Intel's DirectX 12 driver leaks about 90 KB of private memory on every
//! `IDXGISwapChain::Present`, with any swap effect, buffer count or latency
//! setting. A minimal program that only presents an empty D3D12 swapchain
//! reproduces it, so the leak is not in wgpu or MTT. Measured on a UHD 730 with
//! driver 32.0.101.7092, with the monitor on either GPU; NVIDIA does not leak.
//! Vulkan on the same GPU presents without leaking and, once the main window is
//! composed as opaque, reaches the screen at the full refresh rate.
//!
//! The renderer is created before any adapter exists, so the render GPU is
//! predicted from DXGI the same way the adapter selector will choose it.

use super::GpuPreference;

const INTEL_PCI_VENDOR_ID: u32 = 0x8086;

/// Whether the saved renderer preference leaves the choice to MTT. Explicit
/// Vulkan and OpenGL choices are already respected as they are.
fn renderer_is_automatic(renderer_preference: Option<&str>) -> bool {
    !matches!(renderer_preference, Some("vulkan" | "glow" | "gl"))
}

/// Pure decision: try Vulkan before DirectX 12.
fn vulkan_first(renderer_preference: Option<&str>, render_vendor: Option<u32>) -> bool {
    renderer_is_automatic(renderer_preference) && render_vendor == Some(INTEL_PCI_VENDOR_ID)
}

/// PCI vendor of the GPU that `gpu_preference` will render on.
#[cfg(target_os = "windows")]
fn predicted_render_vendor(gpu_preference: GpuPreference) -> Option<u32> {
    use crate::infrastructure::windows::gpu_topology;

    let adapter = match gpu_preference {
        GpuPreference::Automatic => gpu_topology::primary_display_adapter(),
        GpuPreference::HighPerformance => gpu_topology::adapter_by_power_preference(true),
        GpuPreference::LowPower => gpu_topology::adapter_by_power_preference(false),
    };
    adapter.map(|adapter| adapter.vendor_id)
}

#[cfg(not(target_os = "windows"))]
fn predicted_render_vendor(_gpu_preference: GpuPreference) -> Option<u32> {
    None
}

/// Whether an automatic renderer choice should start with Vulkan because the
/// render GPU is Intel.
pub fn prefers_vulkan(renderer_preference: Option<&str>, gpu_preference: GpuPreference) -> bool {
    // Checked first so an explicit choice never queries DXGI.
    renderer_is_automatic(renderer_preference)
        && vulkan_first(renderer_preference, predicted_render_vendor(gpu_preference))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_renderer_on_intel_starts_with_vulkan() {
        assert!(vulkan_first(None, Some(INTEL_PCI_VENDOR_ID)));
        assert!(vulkan_first(Some("auto"), Some(INTEL_PCI_VENDOR_ID)));
        assert!(vulkan_first(Some("dx12"), Some(INTEL_PCI_VENDOR_ID)));
    }

    #[test]
    fn other_vendors_and_unknown_gpus_keep_directx_12() {
        assert!(!vulkan_first(None, Some(0x10de)));
        assert!(!vulkan_first(None, Some(0x1002)));
        assert!(!vulkan_first(None, None));
    }

    #[test]
    fn explicit_renderer_choices_are_kept() {
        assert!(!vulkan_first(Some("vulkan"), Some(INTEL_PCI_VENDOR_ID)));
        assert!(!vulkan_first(Some("glow"), Some(INTEL_PCI_VENDOR_ID)));
        assert!(!vulkan_first(Some("gl"), Some(INTEL_PCI_VENDOR_ID)));
    }
}
