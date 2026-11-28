//! Detects a preferred GPU that does not drive the monitor.
//!
//! When the preferred GPU does not own the monitor's output, Windows copies
//! every presented frame to the GPU that does. On desktops that copy stalls the
//! UI thread for about 500 ms after idle periods, and with some drivers
//! (measured: Intel UHD on DirectX 12 with the monitor on an NVIDIA GPU) every
//! frame also leaks native memory that is never returned. Hybrid laptops may
//! render on the discrete GPU on purpose, so the preference is still honored;
//! the user is only told about the cost and offered the Automatic preference.

use std::sync::OnceLock;

use super::AdapterIdentity;

/// The preferred GPU renders, but another GPU drives the monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayAdapterMismatch {
    /// Adapter the saved preference selected and that renders the window.
    pub preferred: String,
    /// Adapter driving the monitor, which Automatic would select.
    pub display: String,
}

static MISMATCH: OnceLock<DisplayAdapterMismatch> = OnceLock::new();

/// The mismatch found at startup, if the preferred GPU does not drive the
/// monitor.
pub fn display_adapter_mismatch() -> Option<&'static DisplayAdapterMismatch> {
    MISMATCH.get()
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn record_mismatch(value: DisplayAdapterMismatch) {
    let _ = MISMATCH.set(value);
}

/// Index of the monitor's adapter when it differs from `preferred` and is one
/// of the candidates, so that Automatic could select it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn other_display_adapter(
    candidates: &[AdapterIdentity],
    preferred: usize,
    display: Option<(u32, u32)>,
) -> Option<usize> {
    let (vendor, device) = display?;
    let is_display = |c: &AdapterIdentity| c.vendor == vendor && c.device == device;
    if candidates.get(preferred).is_none_or(is_display) {
        return None;
    }
    candidates.iter().position(is_display)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::wgpu;

    const IGPU: (u32, u32) = (0x8086, 0x1111);
    const DGPU: (u32, u32) = (0x10de, 0x2222);

    fn identity((vendor, device): (u32, u32)) -> AdapterIdentity {
        AdapterIdentity {
            vendor,
            device,
            device_type: wgpu::DeviceType::Other,
        }
    }

    #[test]
    fn preferred_gpu_without_the_monitor_reports_the_display_gpu() {
        let candidates = [identity(DGPU), identity(IGPU)];
        assert_eq!(other_display_adapter(&candidates, 1, Some(DGPU)), Some(0));
    }

    #[test]
    fn preferred_gpu_driving_the_monitor_reports_nothing() {
        let candidates = [identity(DGPU), identity(IGPU)];
        assert_eq!(other_display_adapter(&candidates, 1, Some(IGPU)), None);
    }

    #[test]
    fn unknown_or_unavailable_display_gpu_reports_nothing() {
        let candidates = [identity(IGPU)];
        assert_eq!(other_display_adapter(&candidates, 0, None), None);
        assert_eq!(other_display_adapter(&candidates, 0, Some(DGPU)), None);
    }
}
