//! Hides the main window's startup geometry changes.
//!
//! Applying `Maximized(true)` to the hidden startup window makes Windows show
//! it (`ShowWindow(SW_MAXIMIZE)`) at its 800x600 creation size and animate the
//! maximize before it is hidden again and revealed. The window is therefore
//! DWM-cloaked before its first geometry command and uncloaked only after a
//! frame has been painted while visible at the final geometry.
//!
//! With wgpu the frame must also have been presented at the current client
//! size: a pass can run before the swapchain follows the maximize, and with a
//! copy-based present (Vulkan on Intel) the screen would keep showing the
//! smaller startup frame after the reveal.

use crate::app::ImageViewerApp;
use crate::infrastructure::windows::window_cloak::set_window_cloaked;
use eframe::egui;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;

/// Upper bound so a missed state transition can never leave the window invisible.
const MAX_CLOAK_DURATION: Duration = Duration::from_millis(1500);

/// Ready frames needed before uncloaking: the first one is painted at the end
/// of its pass, so the window is uncloaked on the following pass.
const READY_FRAMES_TO_REVEAL: u8 = 2;

pub(crate) struct StartupCloak {
    hwnd: HWND,
    since: Instant,
    expect_maximized: bool,
    ready_frames: u8,
}

impl StartupCloak {
    /// Cloaks `hwnd`. Returns `None` when DWM rejects cloaking.
    pub(crate) fn begin(hwnd: HWND, expect_maximized: bool) -> Option<Self> {
        if !set_window_cloaked(hwnd, true) {
            log::warn!("[STARTUP] DWM cloak unavailable; startup geometry changes may be visible");
            return None;
        }
        Some(Self {
            hwnd,
            since: Instant::now(),
            expect_maximized,
            ready_frames: 0,
        })
    }
}

/// Uncloaks the main window once a frame was painted at the final geometry.
pub fn release_startup_cloak_when_painted(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(cloak) = app.startup_cloak.as_mut() else {
        return;
    };

    // Frames painted before the reveal tick may still use fallback fonts.
    let uses_wgpu = !app.active_gpu_backend.eq_ignore_ascii_case("glow");
    if app.startup_tick >= crate::ui::app::lifecycle::STARTUP_REVEAL_TICK
        && is_at_final_geometry(cloak.hwnd, cloak.expect_maximized)
        && (!uses_wgpu || presented_at_client_size(cloak.hwnd))
    {
        cloak.ready_frames = cloak.ready_frames.saturating_add(1);
    }

    let elapsed = cloak.since.elapsed();
    if !should_release(cloak.ready_frames, elapsed) {
        ctx.request_repaint();
        return;
    }

    if !set_window_cloaked(cloak.hwnd, false) {
        log::warn!("[STARTUP] Failed to uncloak main window");
    }
    log::info!(
        "[STARTUP-TIMELINE] main window uncloaked ready_frames={} cloak_ms={} client={:?} presented={:?} t_ms={}",
        cloak.ready_frames,
        elapsed.as_millis(),
        client_size(cloak.hwnd),
        eframe::egui_wgpu::winit::last_root_present_size(),
        crate::app::startup_timeline::elapsed_ms()
    );
    app.startup_cloak = None;
}

/// Reads the native window state. egui's `ViewportInfo::visible()` cannot be
/// used: winit never reports occlusion on Windows, so it stays `None`.
fn is_at_final_geometry(hwnd: HWND, expect_maximized: bool) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible, IsZoomed};
    unsafe {
        IsWindowVisible(hwnd).as_bool()
            && !IsIconic(hwnd).as_bool()
            && (!expect_maximized || IsZoomed(hwnd).as_bool())
    }
}

fn client_size(hwnd: HWND) -> Option<[u32; 2]> {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.ok()?;
    Some([
        (rect.right - rect.left).max(0) as u32,
        (rect.bottom - rect.top).max(0) as u32,
    ])
}

/// Whether the last frame on screen was rendered at the window's current size.
fn presented_at_client_size(hwnd: HWND) -> bool {
    present_matches_client(
        eframe::egui_wgpu::winit::last_root_present_size(),
        client_size(hwnd),
    )
}

fn present_matches_client(presented: Option<[u32; 2]>, client: Option<[u32; 2]>) -> bool {
    matches!((presented, client), (Some(p), Some(c)) if p == c)
}

fn should_release(ready_frames: u8, elapsed: Duration) -> bool {
    ready_frames >= READY_FRAMES_TO_REVEAL || elapsed >= MAX_CLOAK_DURATION
}

#[cfg(test)]
mod tests {
    use super::{present_matches_client, should_release, MAX_CLOAK_DURATION};
    use std::time::Duration;

    #[test]
    fn waits_for_a_painted_frame_at_final_geometry() {
        assert!(!should_release(0, Duration::ZERO));
        assert!(!should_release(1, Duration::from_millis(10)));
        assert!(should_release(2, Duration::from_millis(10)));
    }

    #[test]
    fn needs_a_present_at_the_current_client_size() {
        assert!(present_matches_client(
            Some([2560, 1369]),
            Some([2560, 1369])
        ));
        assert!(!present_matches_client(
            Some([1280, 720]),
            Some([2560, 1369])
        ));
        assert!(!present_matches_client(None, Some([2560, 1369])));
        assert!(!present_matches_client(Some([2560, 1369]), None));
    }

    #[test]
    fn timeout_always_releases() {
        assert!(should_release(0, MAX_CLOAK_DURATION));
    }
}
