//! Presents one fresh frame after a working-set trim of a visible window.
//!
//! The trim pages out the render path (wgpu upload buffers, swapchain, GPU
//! driver). With the identical-frame skip an idle window presents nothing
//! afterwards, so the first scroll faults all of it back in a single frame and
//! hitches. One present right after the trim series brings those pages back
//! before the next interaction. Backgrounded windows are skipped: they need no
//! pixels, and restoring them already invalidates the presented frame.

use std::sync::OnceLock;

use eframe::egui;

static REPAINT_CTX: OnceLock<egui::Context> = OnceLock::new();

/// Gives the trim thread a way to request the main window repaint.
pub(super) fn register(ctx: &egui::Context) {
    REPAINT_CTX.get_or_init(|| ctx.clone());
}

/// Called once a trim series paged out memory.
pub(super) fn present_after_trim() {
    #[cfg(target_os = "windows")]
    if crate::infrastructure::windows::window_subclass::is_backgrounded() {
        return;
    }
    if let Some(ctx) = REPAINT_CTX.get() {
        eframe::egui_wgpu::winit::invalidate_presented_frames();
        ctx.request_repaint();
    }
}
