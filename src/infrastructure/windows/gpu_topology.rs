//! Which GPU drives which monitor.
//!
//! On a machine with several GPUs, only the adapter that owns a monitor's
//! output can scan out to it directly. Rendering on any other adapter makes
//! Windows copy every presented frame across adapters (shown as the "Copy"
//! engine in Task Manager), which is expensive on desktop systems that are not
//! configured as hybrid (Optimus-style) graphics.

use windows::Win32::Foundation::HWND;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
};
use windows::Win32::Graphics::Gdi::{
    MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTOPRIMARY,
};

/// PCI identity of a hardware adapter as reported by DXGI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayAdapter {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
}

const NVIDIA_PCI_VENDOR_ID: u32 = 0x10DE;

impl DisplayAdapter {
    /// Whether a wgpu adapter (`AdapterInfo::vendor` / `device`) is this adapter.
    pub fn matches(&self, vendor: u32, device: u32) -> bool {
        self.vendor_id == vendor && self.device_id == device
    }

    /// NVIDIA RTX GPUs are the ones that run RTX Video Super Resolution and RTX HDR.
    pub fn is_nvidia_rtx(&self) -> bool {
        self.vendor_id == NVIDIA_PCI_VENDOR_ID && self.name.to_ascii_uppercase().contains("RTX")
    }
}

fn adapter_name(description: &[u16]) -> String {
    let end = description
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(description.len());
    String::from_utf16_lossy(&description[..end])
}

/// Adapter that owns the output of `monitor`, if DXGI reports one.
pub fn display_adapter_for_monitor(monitor: HMONITOR) -> Option<DisplayAdapter> {
    if monitor.is_invalid() {
        return None;
    }
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut adapter_index = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(adapter_index) {
                Ok(adapter) => adapter,
                Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => return None,
                Err(_) => return None,
            };
            adapter_index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let mut output_index = 0u32;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                output_index += 1;
                let Ok(output_desc) = output.GetDesc() else {
                    continue;
                };
                if output_desc.Monitor == monitor {
                    return Some(DisplayAdapter {
                        name: adapter_name(&desc.Description),
                        vendor_id: desc.VendorId,
                        device_id: desc.DeviceId,
                    });
                }
            }
        }
    }
}

/// Adapter driving the primary monitor. Used before the main window exists.
pub fn primary_display_adapter() -> Option<DisplayAdapter> {
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    display_adapter_for_monitor(monitor)
}

/// Adapter driving the monitor that currently holds most of `hwnd`.
pub fn display_adapter_for_window(hwnd: HWND) -> Option<DisplayAdapter> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) };
    display_adapter_for_monitor(monitor)
}

/// Number of hardware (non-software) adapters DXGI reports, or `None` when
/// DXGI is unavailable.
pub fn hardware_adapter_count() -> Option<usize> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut count = 0usize;
        let mut adapter_index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            adapter_index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0 {
                count += 1;
            }
        }
        Some(count)
    }
}

/// Adapter mpv's D3D11 renderer runs on. mpv's `d3d11-adapter` picks the first
/// adapter whose description starts with the requested name (ignoring case);
/// unset, D3D11 uses the default adapter, which is DXGI enumeration index 0.
pub fn mpv_d3d11_adapter(requested: Option<&str>) -> Option<DisplayAdapter> {
    let adapters = unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut adapters = Vec::new();
        let mut adapter_index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            adapter_index += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            adapters.push(DisplayAdapter {
                name: adapter_name(&desc.Description),
                vendor_id: desc.VendorId,
                device_id: desc.DeviceId,
            });
        }
        adapters
    };
    select_mpv_adapter(adapters, requested)
}

fn select_mpv_adapter(
    adapters: Vec<DisplayAdapter>,
    requested: Option<&str>,
) -> Option<DisplayAdapter> {
    match requested.filter(|name| !name.is_empty()) {
        Some(name) => {
            let name = name.to_lowercase();
            adapters
                .into_iter()
                .find(|adapter| adapter.name.to_lowercase().starts_with(&name))
        }
        None => adapters.into_iter().next(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str, vendor_id: u32) -> DisplayAdapter {
        DisplayAdapter {
            name: name.to_string(),
            vendor_id,
            device_id: 1,
        }
    }

    fn intel_then_nvidia() -> Vec<DisplayAdapter> {
        vec![
            adapter("Intel(R) UHD Graphics", 0x8086),
            adapter("NVIDIA GeForce RTX 4060", 0x10DE),
        ]
    }

    #[test]
    fn mpv_uses_the_first_adapter_without_a_requested_name() {
        let selected = select_mpv_adapter(intel_then_nvidia(), None).unwrap();
        assert_eq!(selected.vendor_id, 0x8086);
    }

    #[test]
    fn mpv_matches_the_requested_name_as_a_case_insensitive_prefix() {
        let selected = select_mpv_adapter(intel_then_nvidia(), Some("nvidia geforce")).unwrap();
        assert_eq!(selected.vendor_id, 0x10DE);
        assert!(select_mpv_adapter(intel_then_nvidia(), Some("GeForce")).is_none());
    }

    #[test]
    fn only_nvidia_rtx_adapters_support_rtx_video() {
        assert!(adapter("NVIDIA GeForce RTX 4060", 0x10DE).is_nvidia_rtx());
        assert!(!adapter("NVIDIA GeForce GTX 1660", 0x10DE).is_nvidia_rtx());
        assert!(!adapter("Intel(R) UHD Graphics", 0x8086).is_nvidia_rtx());
    }
}
