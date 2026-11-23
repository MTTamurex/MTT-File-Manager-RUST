use eframe::{egui_wgpu, wgpu};

#[cfg(target_os = "windows")]
use std::sync::Arc;

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
    if renderer_preference == Some("vulkan") || gpu_preference == GpuPreference::Automatic {
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
    fn automatic_delegates_to_wgpu_without_ranking_adapters() {
        assert_eq!(
            GpuPreference::Automatic.power_preference(),
            wgpu::PowerPreference::None
        );
        for renderer in [None, Some("vulkan")] {
            assert!(adapter_selector(renderer, GpuPreference::Automatic, 8192).is_none());
        }
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
