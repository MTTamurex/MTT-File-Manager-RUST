//! Opaque DWM composition for windows created with `transparent = true`.
//!
//! winit implements window transparency on Windows with
//! `DwmEnableBlurBehindWindow` and an empty region, which makes DWM blend the
//! whole window with what is behind it on every frame. The Glow renderer has
//! no per-surface alpha mode (unlike the wgpu swapchain), so the main window
//! has to drop the blur-behind itself to be composed as opaque.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DwmEnableBlurBehindWindow, DWM_BB_ENABLE, DWM_BLURBEHIND};

/// Set to `1` to keep the Glow main window alpha-composited (measurement only).
pub const GL_ROOT_ALPHA_ENV: &str = "MTT_GL_ROOT_ALPHA";

/// Disable the blur-behind that winit enabled for a transparent window, so DWM
/// composes it as an opaque surface. Best-effort: failures are only logged.
pub fn disable_blur_behind(hwnd: HWND) {
    if hwnd.is_invalid() {
        return;
    }
    let blur_behind = DWM_BLURBEHIND {
        dwFlags: DWM_BB_ENABLE,
        fEnable: false.into(),
        ..Default::default()
    };
    if let Err(err) = unsafe { DwmEnableBlurBehindWindow(hwnd, &blur_behind) } {
        log::warn!("[GPU] failed to make the main window opaque for DWM: {err}");
    }
}
