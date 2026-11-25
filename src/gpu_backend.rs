use eframe::{egui_wgpu, wgpu};

#[cfg(target_os = "windows")]
use std::sync::Arc;

pub mod tuning;

pub const PREFERENCE_KEY: &str = "gpu_adapter_preference";

pub const WGPU_REQUIRED_MAX_TEXTURE_DIMENSION_2D: u32 = 8192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuPreference {
    /// Preserve the previous behavior for existing installations.
    #[default]
    HighPerformance,
    LowPower,
    Automatic,
}

impl GpuPreference {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("low_power") => Self::LowPower,
            Some("automatic") => Self::Automatic,
            _ => Self::HighPerformance,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::HighPerformance => "high_performance",
            Self::LowPower => "low_power",
            Self::Automatic => "automatic",
        }
    }

    pub fn power_preference(self) -> wgpu::PowerPreference {
        match self {
            Self::HighPerformance => wgpu::PowerPreference::HighPerformance,
            Self::LowPower => wgpu::PowerPreference::LowPower,
            Self::Automatic => wgpu::PowerPreference::None,
        }
    }
}

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

pub fn adapter_selector(
    renderer_preference: Option<&str>,
    gpu_preference: GpuPreference,
    required_texture_dimension: u32,
) -> Option<egui_wgpu::NativeAdapterSelectorMethod> {
    if gpu_preference == GpuPreference::Automatic {
        // Works for DX12 and Vulkan alike: both report PCI vendor/device ids.
        #[cfg(target_os = "windows")]
        {
            return Some(display_aware_adapter_selector(required_texture_dimension));
        }
        #[cfg(not(target_os = "windows"))]
        {
            return None;
        }
    }
    if renderer_preference == Some("vulkan") {
        return None;
    }

    #[cfg(target_os = "windows")]
    {
        Some(windows_gpu_adapter_selector(
            gpu_preference,
            required_texture_dimension,
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (gpu_preference, required_texture_dimension);
        None
    }
}

#[cfg(target_os = "windows")]
fn windows_gpu_adapter_selector(
    preference: GpuPreference,
    required_texture_dimension: u32,
) -> egui_wgpu::NativeAdapterSelectorMethod {
    Arc::new(move |adapters, compatible_surface| {
        let adapter = adapters
            .iter()
            .filter(|adapter| {
                let info = adapter.get_info();
                info.backend == wgpu::Backend::Dx12
                    && match compatible_surface {
                        Some(surface) => adapter.is_surface_supported(surface),
                        None => true,
                    }
                    && adapter.limits().max_texture_dimension_2d >= required_texture_dimension
            })
            .min_by_key(|adapter| device_type_rank(adapter.get_info().device_type, preference));

        if let Some(adapter) = adapter {
            log::info!(
                "[STARTUP] GPU adapter selected for preference {}: {}",
                preference.key(),
                adapter_summary(adapter)
            );
            return Ok(adapter.clone());
        }

        Err(format!(
            "no compatible DirectX 12 adapter matched GPU preference {}; available adapters: {}",
            preference.key(),
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

/// Automatic: render on the GPU that drives the monitor, so presented frames
/// never need a cross-adapter copy. Without topology information, keep wgpu's
/// enumeration order (the previous `PowerPreference::None` behavior).
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
        if let Some(index) = automatic_adapter_index(&identities, display_ids) {
            let adapter = candidates[index];
            log::info!(
                "[STARTUP] GPU adapter selected for preference automatic (display adapter: {}): {}",
                display
                    .as_ref()
                    .map(|d| d.name.as_str())
                    .unwrap_or("unknown"),
                adapter_summary(adapter)
            );
            return Ok(adapter.clone());
        }
        Err(format!(
            "no compatible adapter for GPU preference automatic; available adapters: {}",
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
fn automatic_adapter_index(
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

#[cfg(any(target_os = "windows", test))]
fn device_type_rank(device_type: wgpu::DeviceType, preference: GpuPreference) -> u8 {
    match preference {
        GpuPreference::HighPerformance => match device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            wgpu::DeviceType::Other => 3,
            wgpu::DeviceType::Cpu => 4,
        },
        GpuPreference::LowPower => match device_type {
            wgpu::DeviceType::IntegratedGpu => 0,
            wgpu::DeviceType::DiscreteGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            wgpu::DeviceType::Other => 3,
            wgpu::DeviceType::Cpu => 4,
        },
        GpuPreference::Automatic => 0,
    }
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
/// ("Composed: Copy with GPU GDI" in PresentMon). Measured on a UHD 730: ~30
/// of 60 frames reached the screen and DWM spent ~10 ms per frame copying,
/// while DirectX 12 on the same GPU used hardware independent flip.
pub fn vulkan_presents_by_copy(backend: wgpu::Backend, vendor: u32) -> bool {
    backend == wgpu::Backend::Vulkan && vendor == INTEL_PCI_VENDOR_ID
}

static RENDER_ADAPTER_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Name of the wgpu adapter rendering the main window, once it is known.
pub fn render_adapter_name() -> Option<&'static str> {
    RENDER_ADAPTER_NAME.get().map(String::as_str)
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
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    if let Some(render_state) = &cc.wgpu_render_state {
        let _ = RENDER_ADAPTER_NAME.set(render_state.adapter.get_info().name);
    }
    let hwnd = cc
        .window_handle()
        .ok()
        .and_then(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(handle) => {
                Some(windows::Win32::Foundation::HWND(handle.hwnd.get() as _))
            }
            _ => None,
        });
    let Some(display) = hwnd
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
    fn gpu_preference_defaults_to_high_performance_for_existing_installations() {
        assert_eq!(GpuPreference::parse(None), GpuPreference::HighPerformance);
        assert_eq!(
            GpuPreference::parse(Some("invalid")),
            GpuPreference::HighPerformance
        );
    }

    #[test]
    fn gpu_preference_round_trips_supported_values() {
        for preference in [
            GpuPreference::HighPerformance,
            GpuPreference::LowPower,
            GpuPreference::Automatic,
        ] {
            assert_eq!(GpuPreference::parse(Some(preference.key())), preference);
        }
    }

    #[test]
    fn low_power_prefers_integrated_but_can_fall_back_to_discrete() {
        assert!(
            device_type_rank(wgpu::DeviceType::IntegratedGpu, GpuPreference::LowPower)
                < device_type_rank(wgpu::DeviceType::DiscreteGpu, GpuPreference::LowPower)
        );
    }

    #[test]
    fn high_performance_keeps_discrete_gpu_priority() {
        assert!(
            device_type_rank(
                wgpu::DeviceType::DiscreteGpu,
                GpuPreference::HighPerformance
            ) < device_type_rank(
                wgpu::DeviceType::IntegratedGpu,
                GpuPreference::HighPerformance
            )
        );
    }

    #[test]
    fn automatic_uses_display_aware_selector_on_every_backend() {
        assert_eq!(
            GpuPreference::Automatic.power_preference(),
            wgpu::PowerPreference::None
        );
        for renderer in [None, Some("vulkan")] {
            assert_eq!(
                adapter_selector(renderer, GpuPreference::Automatic, 8192).is_some(),
                cfg!(target_os = "windows")
            );
        }
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
    fn automatic_prefers_the_adapter_driving_the_display() {
        let candidates = [
            identity(NVIDIA, wgpu::DeviceType::DiscreteGpu),
            identity(INTEL, wgpu::DeviceType::IntegratedGpu),
        ];
        assert_eq!(automatic_adapter_index(&candidates, Some(INTEL)), Some(1));
        assert_eq!(automatic_adapter_index(&candidates, Some(NVIDIA)), Some(0));
    }

    #[test]
    fn automatic_keeps_enumeration_order_without_display_information() {
        let candidates = [
            identity((0x1414, 0x8c), wgpu::DeviceType::Cpu),
            identity(INTEL, wgpu::DeviceType::IntegratedGpu),
            identity(NVIDIA, wgpu::DeviceType::DiscreteGpu),
        ];
        assert_eq!(automatic_adapter_index(&candidates, None), Some(1));
        // A display adapter that is not a candidate (e.g. unsupported surface).
        assert_eq!(
            automatic_adapter_index(&candidates, Some((0x1002, 0x1))),
            Some(1)
        );
        assert_eq!(automatic_adapter_index(&[], Some(INTEL)), None);
    }

    #[test]
    fn low_power_keeps_hardware_fallback_ahead_of_software_adapters() {
        let candidates = [wgpu::DeviceType::Cpu, wgpu::DeviceType::DiscreteGpu];
        assert_eq!(
            candidates
                .into_iter()
                .min_by_key(|device| device_type_rank(*device, GpuPreference::LowPower)),
            Some(wgpu::DeviceType::DiscreteGpu)
        );
    }
}
