//! Presentation tuning knobs read from environment variables.
//!
//! These exist to measure which part of the wgpu presentation path costs frame
//! time when the monitor is driven by an integrated GPU. Every default matches
//! the behavior before the knobs existed. Child viewer processes inherit the
//! environment, so one launch configures the manager and its viewers alike.

use eframe::{egui_wgpu, wgpu};

pub const SWAPCHAIN_ENV: &str = "MTT_DX12_SWAPCHAIN";
pub const TRANSPARENT_ENV: &str = "MTT_MAIN_TRANSPARENT";
pub const LATENCY_ENV: &str = "MTT_WGPU_LATENCY";
pub const PRESENT_ENV: &str = "MTT_WGPU_PRESENT";
pub const MEMORY_HINT_ENV: &str = "MTT_WGPU_MEMHINT";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dx12Swapchain {
    /// DirectComposition visual on a window that still has a redirection bitmap.
    Visual,
    /// DirectComposition visual on a `WS_EX_NOREDIRECTIONBITMAP` window.
    VisualNoRedirection,
    /// Classic flip-model swapchain bound to the HWND.
    Hwnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentTuning {
    pub swapchain: Dx12Swapchain,
    /// Main window only; viewers never use transparency.
    pub main_window_transparent: bool,
    pub frame_latency: u32,
    pub present_mode: wgpu::PresentMode,
    pub memory_hints_performance: bool,
}

impl Default for PresentTuning {
    fn default() -> Self {
        Self {
            swapchain: Dx12Swapchain::Visual,
            main_window_transparent: true,
            frame_latency: 2,
            present_mode: wgpu::PresentMode::AutoVsync,
            memory_hints_performance: false,
        }
    }
}

impl PresentTuning {
    pub fn from_env() -> Self {
        Self::parse(|name| std::env::var(name).ok())
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>) -> Self {
        let defaults = Self::default();
        let value = |name: &str| get(name).map(|v| v.trim().to_ascii_lowercase());
        Self {
            swapchain: match value(SWAPCHAIN_ENV).as_deref() {
                Some("visual_noredir") => Dx12Swapchain::VisualNoRedirection,
                Some("hwnd") => Dx12Swapchain::Hwnd,
                _ => defaults.swapchain,
            },
            main_window_transparent: !matches!(value(TRANSPARENT_ENV).as_deref(), Some("0")),
            frame_latency: match value(LATENCY_ENV).as_deref() {
                Some("1") => 1,
                Some("3") => 3,
                _ => defaults.frame_latency,
            },
            // Only modes wgpu always accepts: Fifo is mandatory, and the Auto
            // modes fall back to it instead of failing surface configuration.
            present_mode: match value(PRESENT_ENV).as_deref() {
                Some("fifo") => wgpu::PresentMode::Fifo,
                Some("auto_novsync") => wgpu::PresentMode::AutoNoVsync,
                _ => defaults.present_mode,
            },
            memory_hints_performance: matches!(
                value(MEMORY_HINT_ENV).as_deref(),
                Some("performance")
            ),
        }
    }

    pub fn dx12_presentation_system(self) -> wgpu::Dx12SwapchainKind {
        match self.swapchain {
            Dx12Swapchain::Visual | Dx12Swapchain::VisualNoRedirection => {
                wgpu::Dx12SwapchainKind::DxgiFromVisual
            }
            Dx12Swapchain::Hwnd => wgpu::Dx12SwapchainKind::DxgiFromHwnd,
        }
    }

    /// Whether DX12 windows should be created without a redirection bitmap.
    pub fn dx12_without_redirection_bitmap(self) -> bool {
        self.swapchain == Dx12Swapchain::VisualNoRedirection
    }

    pub fn surface_config(self) -> egui_wgpu::SurfaceConfig {
        egui_wgpu::SurfaceConfig {
            present_mode: self.present_mode,
            desired_maximum_frame_latency: Some(self.frame_latency),
        }
    }

    pub fn memory_hints(self) -> wgpu::MemoryHints {
        if self.memory_hints_performance {
            wgpu::MemoryHints::Performance
        } else {
            wgpu::MemoryHints::MemoryUsage
        }
    }

    pub fn summary(self) -> String {
        format!(
            "swapchain={:?} main_transparent={} frame_latency={} present_mode={:?} memory_hints={}",
            self.swapchain,
            self.main_window_transparent,
            self.frame_latency,
            self.present_mode,
            if self.memory_hints_performance {
                "performance"
            } else {
                "memory_usage"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_previous_behavior() {
        let tuning = PresentTuning::parse(|_| None);
        assert_eq!(tuning, PresentTuning::default());
        assert_eq!(
            tuning.dx12_presentation_system(),
            wgpu::Dx12SwapchainKind::DxgiFromVisual
        );
        assert!(!tuning.dx12_without_redirection_bitmap());
        assert_eq!(
            tuning.surface_config(),
            egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT
        );
        assert!(matches!(
            tuning.memory_hints(),
            wgpu::MemoryHints::MemoryUsage
        ));
    }

    #[test]
    fn parses_every_knob() {
        let tuning = PresentTuning::parse(|name| {
            Some(
                match name {
                    SWAPCHAIN_ENV => "HWND",
                    TRANSPARENT_ENV => "0",
                    LATENCY_ENV => "1",
                    PRESENT_ENV => "auto_novsync",
                    MEMORY_HINT_ENV => "performance",
                    _ => return None,
                }
                .to_string(),
            )
        });
        assert_eq!(tuning.swapchain, Dx12Swapchain::Hwnd);
        assert!(!tuning.main_window_transparent);
        assert_eq!(tuning.frame_latency, 1);
        assert_eq!(tuning.present_mode, wgpu::PresentMode::AutoNoVsync);
        assert!(matches!(
            tuning.memory_hints(),
            wgpu::MemoryHints::Performance
        ));
    }

    #[test]
    fn unknown_values_keep_defaults() {
        let tuning = PresentTuning::parse(|_| Some("bogus".to_string()));
        assert_eq!(tuning, PresentTuning::default());
    }
}
