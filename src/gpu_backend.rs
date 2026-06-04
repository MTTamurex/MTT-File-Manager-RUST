use eframe::{egui_wgpu, wgpu};

#[cfg(target_os = "windows")]
use std::sync::Arc;

pub(crate) const WGPU_REQUIRED_MAX_TEXTURE_DIMENSION_2D: u32 = 8192;

/// Convert a stored backend preference string into wgpu::Backends flags.
pub(crate) fn parse_gpu_backend_preference(pref: Option<&str>) -> wgpu::Backends {
    match pref {
        Some("dx12") => wgpu::Backends::DX12,
        Some("vulkan") => wgpu::Backends::VULKAN,
        Some("gl") => wgpu::Backends::GL,
        _ => auto_gpu_backends(),
    }
}

#[cfg(target_os = "windows")]
fn auto_gpu_backends() -> wgpu::Backends {
    wgpu::Backends::VULKAN | wgpu::Backends::GL | wgpu::Backends::DX12
}

#[cfg(not(target_os = "windows"))]
fn auto_gpu_backends() -> wgpu::Backends {
    wgpu::Backends::PRIMARY | wgpu::Backends::GL
}

pub(crate) fn adapter_selector(
    pref: Option<&str>,
) -> Option<egui_wgpu::NativeAdapterSelectorMethod> {
    match pref {
        Some("dx12") | Some("vulkan") | Some("gl") => None,
        _ => {
            #[cfg(target_os = "windows")]
            {
                Some(auto_gpu_adapter_selector())
            }
            #[cfg(not(target_os = "windows"))]
            {
                None
            }
        }
    }
}

#[cfg(target_os = "windows")]
const AUTO_BACKEND_PRIORITY: &[wgpu::Backend] = &[
    wgpu::Backend::Vulkan,
    wgpu::Backend::Gl,
    wgpu::Backend::Dx12,
];

#[cfg(target_os = "windows")]
fn backend_priority_label(order: &[wgpu::Backend]) -> String {
    order
        .iter()
        .map(|backend| format!("{backend:?}"))
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[cfg(target_os = "windows")]
fn adapter_summary(adapter: &wgpu::Adapter) -> String {
    let info = adapter.get_info();
    format!(
        "backend: {:?}, device_type: {:?}, name: {:?}, driver: {:?} {}",
        info.backend, info.device_type, info.name, info.driver, info.driver_info
    )
}

#[cfg(target_os = "windows")]
fn device_type_rank(device_type: wgpu::DeviceType) -> u8 {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

#[cfg(target_os = "windows")]
fn auto_gpu_adapter_selector() -> egui_wgpu::NativeAdapterSelectorMethod {
    Arc::new(|adapters, compatible_surface| {
        let priority = AUTO_BACKEND_PRIORITY;

        for preferred_backend in priority {
            let adapter = adapters
                .iter()
                .filter(|adapter| {
                    let info = adapter.get_info();
                    if info.backend != *preferred_backend {
                        return false;
                    }

                    match compatible_surface {
                        Some(surface) if !adapter.is_surface_supported(surface) => false,
                        _ => {
                            adapter.limits().max_texture_dimension_2d
                                >= WGPU_REQUIRED_MAX_TEXTURE_DIMENSION_2D
                        }
                    }
                })
                // native_adapter_selector bypasses wgpu's PowerPreference, so keep
                // the original high-performance intent within each backend.
                .min_by_key(|adapter| device_type_rank(adapter.get_info().device_type));

            if let Some(adapter) = adapter {
                log::info!(
                    "[STARTUP] Auto GPU backend selected from priority {}: {}",
                    backend_priority_label(priority),
                    adapter_summary(adapter)
                );
                return Ok(adapter.clone());
            }
        }

        Err(format!(
            "no compatible GPU adapter matched priority {}; available adapters: {}",
            backend_priority_label(priority),
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
