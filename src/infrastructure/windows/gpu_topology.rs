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

impl DisplayAdapter {
    /// Whether a wgpu adapter (`AdapterInfo::vendor` / `device`) is this adapter.
    pub fn matches(&self, vendor: u32, device: u32) -> bool {
        self.vendor_id == vendor && self.device_id == device
    }
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
                    let end = desc
                        .Description
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(desc.Description.len());
                    return Some(DisplayAdapter {
                        name: String::from_utf16_lossy(&desc.Description[..end]),
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
