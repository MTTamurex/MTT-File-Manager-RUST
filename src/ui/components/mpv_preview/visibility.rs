use super::*;

impl MpvPreview {
    /// Temporarily hides the video surface (for popups over the video area).
    pub fn hide_for_overlay(&mut self) {
        self.surface.set_visible(false);
    }

    /// Restores the video surface without bypassing first-frame readiness.
    pub fn restore_from_overlay(&mut self) {
        self.apply_surface_visibility();
    }

    pub fn set_visibility(&mut self, visible: bool) {
        self.is_visible = visible;
        self.apply_surface_visibility();
    }

    fn apply_surface_visibility(&self) {
        // MPV creates its own visible D3D11 child inside our host. A black host
        // alone cannot cover that uninitialized child; keep the whole subtree
        // hidden until MPV reports that playback has restarted on its first frame.
        self.surface
            .set_visible(self.is_visible && self.first_frame_ready.load(Ordering::Acquire));
    }

    pub(crate) fn set_overlay_rects(
        &mut self,
        overlays: &[crate::ui::video_overlay::VideoOverlay],
        pixels_per_point: f32,
    ) {
        if self.is_docked() {
            self.surface.set_overlay_rects(overlays, pixels_per_point);
        } else {
            self.surface.set_overlay_rects(&[], pixels_per_point);
        }
    }
}
