use super::*;

impl IconLoader {
    /// Ensures the computer icon texture is loaded.
    ///
    /// Extracts from the Windows Shell once per session. It is not persisted so
    /// theme and shell icon changes are reflected on the next launch.
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        if self.computer_icon_texture.is_some() {
            return;
        }

        if let Ok((data, width, height)) = windows::extract_computer_icon(IconSize::Jumbo) {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &data);
            self.computer_icon_texture =
                Some(ctx.load_texture("computer_icon", image, egui::TextureOptions::LINEAR));
        }
    }

    /// Gets the computer icon texture (must call ensure_computer_icon first).
    pub fn computer_icon(&self) -> Option<&egui::TextureHandle> {
        self.computer_icon_texture.as_ref()
    }

    /// Ensures the recycle bin icon texture is loaded and returns it.
    ///
    /// Extracts from the Windows Shell once per session and does not persist it.
    pub fn ensure_recycle_bin_icon(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if let Some(tex) = &self.recycle_bin_icon_texture {
            return Some(tex.clone());
        }

        if let Ok((data, width, height)) = windows::extract_recycle_bin_icon(IconSize::Jumbo) {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &data);
            let texture = ctx.load_texture("recycle_bin_icon", image, egui::TextureOptions::LINEAR);
            self.recycle_bin_icon_texture = Some(texture.clone());
            return Some(texture);
        }

        None
    }
}
