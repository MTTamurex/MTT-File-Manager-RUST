use eframe::{egui_wgpu, wgpu};

#[cfg(target_os = "windows")]
use std::sync::Arc;

pub mod glow_pacing;
pub mod intel_vulkan;
pub mod tuning;

pub const WGPU_REQUIRED_MAX_TEXTURE_DIMENSION_2D: u32 = 8192;

/// Convert a stored renderer preference into wgpu backend flags.
pub fn parse_renderer_preference(pref: Option<&str>) -> wgpu::Backends {
    match pref {
        Some("vulkan") => wgpu::Backends::VULKAN,
        _ => auto_gpu_backends(),
    }
}

#[cfg(target_os = "windows")]
fn auto_gpu_backends() -> wgpu::Backends {
    wgpu::Backends::DX12
}

#[cfg(not(target_os = "windows"))]
fn auto_gpu_backends() -> wgpu::Backends {
    wgpu::Backends::PRIMARY
}

/// Adapter selector for the main window and the viewers: always the GPU that
/// drives the monitor. Rendering on any other GPU makes Windows copy every
/// presented frame between GPUs, which measured worse in every multi-GPU setup
/// (UI stalls, extra latency, and the other GPU kept awake), so there is no
/// user-facing GPU choice. Works for DX12 and Vulkan alike: both report PCI
/// vendor/device ids.
pub fn adapter_selector(
    required_texture_dimension: u32,
) -> Option<egui_wgpu::NativeAdapterSelectorMethod> {
    #[cfg(target_os = "windows")]
    {
        Some(display_aware_adapter_selector(required_texture_dimension))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = required_texture_dimension;
        None
    }
}

/// Without topology information, keep wgpu's enumeration order.
#[cfg(target_os = "windows")]
fn display_aware_adapter_selector(
    required_texture_dimension: u32,
) -> egui_wgpu::NativeAdapterSelectorMethod {
    Arc::new(move |adapters, compatible_surface| {
        let display = crate::infrastructure::windows::gpu_topology::primary_display_adapter();
        let candidates: Vec<&wgpu::Adapter> = adapters
            .iter()
            .filter(|adapter| {
                compatible_surface.is_none_or(|surface| adapter.is_surface_supported(surface))
                    && adapter.limits().max_texture_dimension_2d >= required_texture_dimension
            })
            .collect();
        let identities: Vec<AdapterIdentity> = candidates
            .iter()
            .map(|adapter| {
                let info = adapter.get_info();
                AdapterIdentity {
                    vendor: info.vendor,
                    device: info.device,
                    device_type: info.device_type,
                }
            })
            .collect();
        let display_ids = display.as_ref().map(|d| (d.vendor_id, d.device_id));
        if let Some(index) = display_adapter_index(&identities, display_ids) {
            let adapter = candidates[index];
            log::info!(
                "[STARTUP] GPU adapter selected (display adapter: {}): {}",
                display
                    .as_ref()
                    .map(|d| d.name.as_str())
                    .unwrap_or("unknown"),
                adapter_summary(adapter)
            );
            return Ok(adapter.clone());
        }
        Err(format!(
            "no compatible GPU adapter; available adapters: {}",
            if adapters.is_empty() {
                "(none)".to_string()
            } else {
                adapters
                    .iter()
                    .map(adapter_summary)
                    .collect::<Vec<_>>()
                    .join("; ")
            }
        ))
    })
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
struct AdapterIdentity {
    vendor: u32,
    device: u32,
    device_type: wgpu::DeviceType,
}

/// Index of the adapter that drives the display; otherwise the first hardware
/// adapter in enumeration order; software adapters only as a last resort.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn display_adapter_index(
    candidates: &[AdapterIdentity],
    display: Option<(u32, u32)>,
) -> Option<usize> {
    if let Some((vendor, device)) = display {
        if let Some(index) = candidates
            .iter()
            .position(|c| c.vendor == vendor && c.device == device)
        {
            return Some(index);
        }
    }
    candidates
        .iter()
        .position(|c| c.device_type != wgpu::DeviceType::Cpu)
        .or_else(|| (!candidates.is_empty()).then_some(0))
}

#[cfg(target_os = "windows")]
fn adapter_summary(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!(
        "backend: {:?}, device_type: {:?}, name: {:?}, driver: {:?} {}",
        info.backend, info.device_type, info.name, info.driver, info.driver_info
    )
}

const INTEL_PCI_VENDOR_ID: u32 = 0x8086;

/// Intel's Windows Vulkan driver presents windowed swapchains by GDI copy
/// ("Composed: Copy with GPU GDI" in PresentMon), while DirectX 12 on the same
/// GPU uses hardware independent flip. Measured on a UHD 730 driving the
/// monitor: with the main window alpha-composited by DWM, 15% of the frames
/// never reached the screen; composed as opaque (`make_root_window_opaque`),
/// all of them did. DirectX 12 is not an alternative there: its present leaks
/// memory (see `intel_vulkan`).
pub fn vulkan_presents_by_copy(backend: wgpu::Backend, vendor: u32) -> bool {
    backend == wgpu::Backend::Vulkan && vendor == INTEL_PCI_VENDOR_ID
}

/// Log which GPU drives the window's monitor versus which GPU renders, and
/// return the display GPU's name when they differ (every frame is then copied
/// between adapters before it reaches the screen).
#[cfg(target_os = "windows")]
pub fn log_display_adapter_topology(
    cc: &eframe::CreationContext<'_>,
    active_gpu_device: &str,
) -> Option<String> {
    use crate::infrastructure::windows::gpu_topology;

    let Some(display) = root_hwnd(cc)
        .and_then(gpu_topology::display_adapter_for_window)
        .or_else(gpu_topology::primary_display_adapter)
    else {
        log::info!("[GPU] display adapter: unknown (no DXGI output matched the monitor)");
        return None;
    };

    let cross_adapter = match &cc.wgpu_render_state {
        Some(render_state) => {
            let info = render_state.adapter.get_info();
            !display.matches(info.vendor, info.device)
        }
        // OpenGL only exposes the renderer string, e.g. "NVIDIA GeForce RTX 4090/PCIe/SSE2".
        None => !gl_renderer_matches(active_gpu_device, &display.name),
    };
    let message = format!(
        "[GPU] display adapter: {} (vendor=0x{:04x} device=0x{:04x}), render adapter: {}, cross_adapter: {}",
        display.name, display.vendor_id, display.device_id, active_gpu_device, cross_adapter
    );
    if cross_adapter {
        log::warn!("{message} - every presented frame is copied between GPUs");
        Some(display.name)
    } else {
        log::info!("{message}");
        None
    }
}

#[cfg(target_os = "windows")]
fn root_hwnd(cc: &eframe::CreationContext<'_>) -> Option<windows::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    cc.window_handle()
        .ok()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => {
                Some(windows::Win32::Foundation::HWND(handle.hwnd.get() as _))
            }
            _ => None,
        })
}

/// The root window is created transparent so the tab-drag preview viewport can
/// be alpha-composited, which makes winit enable DWM blur-behind on it. DWM then
/// alpha-blends the whole main window every frame. The main UI paints an opaque
/// background, so drop the blur-behind when it costs frames:
/// - Glow has no per-surface alpha mode, and on an integrated GPU the blend
///   costs the frame rate while scrolling.
/// - Vulkan on Intel presents by GDI copy; with the blend, DWM saturated a UHD
///   730 and 15% of the frames never reached the screen (0% without it).
///
/// DirectX 12 presents by flip through an opaque swapchain (see
/// vendor/egui-wgpu/PATCHES.md), so it keeps the window as created.
#[cfg(target_os = "windows")]
pub fn make_root_window_opaque(cc: &eframe::CreationContext<'_>, renderer: &str) {
    use crate::infrastructure::windows::window_opacity;

    if std::env::var(window_opacity::ROOT_ALPHA_ENV).as_deref() == Ok("1") {
        log::info!(
            "[GPU] {renderer} main window keeps DWM alpha composition ({}=1)",
            window_opacity::ROOT_ALPHA_ENV
        );
        return;
    }
    match root_hwnd(cc) {
        Some(hwnd) => {
            window_opacity::disable_blur_behind(hwnd);
            log::info!("[GPU] {renderer} main window composed as opaque by DWM");
        }
        None => {
            log::warn!("[GPU] {renderer} main window handle unavailable; DWM alpha stays enabled")
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn log_display_adapter_topology(
    _cc: &eframe::CreationContext<'_>,
    _active_gpu_device: &str,
) -> Option<String> {
    None
}

/// Whether an OpenGL renderer string names the DXGI adapter. An empty renderer
/// string (unknown) is treated as a match so no false warning is shown.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn gl_renderer_matches(renderer: &str, dxgi_name: &str) -> bool {
    let renderer = renderer
        .split('/')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let dxgi_name = dxgi_name.trim().to_ascii_lowercase();
    renderer.is_empty()
        || dxgi_name.is_empty()
        || renderer.contains(&dxgi_name)
        || dxgi_name.contains(&renderer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_aware_selector_is_used_on_windows() {
        assert_eq!(
            adapter_selector(8192).is_some(),
            cfg!(target_os = "windows")
        );
    }

    #[test]
    fn only_vulkan_on_intel_is_flagged_as_copy_presentation() {
        assert!(vulkan_presents_by_copy(wgpu::Backend::Vulkan, 0x8086));
        assert!(!vulkan_presents_by_copy(wgpu::Backend::Dx12, 0x8086));
        assert!(!vulkan_presents_by_copy(wgpu::Backend::Vulkan, 0x10de));
    }

    #[test]
    fn gl_renderer_names_are_matched_against_dxgi_descriptions() {
        assert!(gl_renderer_matches(
            "NVIDIA GeForce RTX 4090/PCIe/SSE2",
            "NVIDIA GeForce RTX 4090"
        ));
        assert!(gl_renderer_matches(
            "Intel(R) UHD Graphics 770",
            "Intel(R) UHD Graphics 770"
        ));
        assert!(!gl_renderer_matches(
            "NVIDIA GeForce RTX 4090/PCIe/SSE2",
            "Intel(R) UHD Graphics 770"
        ));
        assert!(gl_renderer_matches("", "Intel(R) UHD Graphics 770"));
    }

    const NVIDIA: (u32, u32) = (0x10de, 0x2684);
    const INTEL: (u32, u32) = (0x8086, 0xa780);

    fn identity((vendor, device): (u32, u32), device_type: wgpu::DeviceType) -> AdapterIdentity {
        AdapterIdentity {
            vendor,
            device,
            device_type,
        }
    }

    #[test]
    fn selector_prefers_the_adapter_driving_the_display() {
        let candidates = [
            identity(NVIDIA, wgpu::DeviceType::DiscreteGpu),
            identity(INTEL, wgpu::DeviceType::IntegratedGpu),
        ];
        assert_eq!(display_adapter_index(&candidates, Some(INTEL)), Some(1));
        assert_eq!(display_adapter_index(&candidates, Some(NVIDIA)), Some(0));
    }

    #[test]
    fn selector_keeps_enumeration_order_without_display_information() {
        let candidates = [
            identity((0x1414, 0x8c), wgpu::DeviceType::Cpu),
            identity(INTEL, wgpu::DeviceType::IntegratedGpu),
            identity(NVIDIA, wgpu::DeviceType::DiscreteGpu),
        ];
        assert_eq!(display_adapter_index(&candidates, None), Some(1));
        // A display adapter that is not a candidate (e.g. unsupported surface).
        assert_eq!(
            display_adapter_index(&candidates, Some((0x1002, 0x1))),
            Some(1)
        );
        assert_eq!(display_adapter_index(&[], Some(INTEL)), None);
    }
}
