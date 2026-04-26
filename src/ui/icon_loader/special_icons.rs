use super::*;

const RECYCLE_BIN_CACHE_KEY: &str = "shell:recyclebin:v2";

impl IconLoader {
    /// Ensures the computer icon texture is loaded.
    ///
    /// Checks SQLite cache first for instant load; on miss extracts via Shell
    /// API and persists.  A background revalidation thread ensures correctness
    /// after theme changes.
    pub fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        if self.computer_icon_texture.is_some() {
            return;
        }

        // Try SQLite cache first.
        if let Some(dc) = &self.disk_cache {
            if let Some((data, width, height)) = dc.get_shell_icon("shell:computer") {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &data,
                );
                self.computer_icon_texture =
                    Some(ctx.load_texture("computer_icon", image, egui::TextureOptions::LINEAR));

                // Revalidate in background — if the icon changed, update SQLite + texture.
                let tx = self.icon_result_tx.clone();
                let dc = dc.clone();
                let cached_pixels = data;
                std::thread::spawn(move || {
                    let _com = super::ComStaGuard::new();
                    if let Ok((fresh, w, h)) = windows::extract_computer_icon(IconSize::Jumbo) {
                        if fresh != cached_pixels || w != width || h != height {
                            dc.put_shell_icon("shell:computer", &fresh, w, h);
                            let _ = tx.send(AsyncIconResult {
                                key: "__computer__".to_string(),
                                data: Some((fresh, w, h)),
                            });
                        }
                    }
                });
                return;
            }
        }

        // Cache miss — extract synchronously (first launch only).
        if let Ok((data, width, height)) = windows::extract_computer_icon(IconSize::Jumbo) {
            if let Some(dc) = &self.disk_cache {
                dc.put_shell_icon("shell:computer", &data, width, height);
            }
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
    /// Checks SQLite cache first; on miss extracts via Shell API and persists.
    pub fn ensure_recycle_bin_icon(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if let Some(tex) = &self.recycle_bin_icon_texture {
            return Some(tex.clone());
        }

        // Try SQLite cache first.
        if let Some(dc) = &self.disk_cache {
            if let Some((data, width, height)) = dc.get_shell_icon(RECYCLE_BIN_CACHE_KEY) {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [width as usize, height as usize],
                    &data,
                );
                let texture =
                    ctx.load_texture("recycle_bin_icon", image, egui::TextureOptions::LINEAR);
                self.recycle_bin_icon_texture = Some(texture.clone());

                // Revalidate in background.
                let tx = self.icon_result_tx.clone();
                let dc = dc.clone();
                let cached_pixels = data;
                std::thread::spawn(move || {
                    let _com = super::ComStaGuard::new();
                    if let Ok((fresh, w, h)) = windows::extract_recycle_bin_icon(IconSize::Jumbo) {
                        if fresh != cached_pixels || w != width || h != height {
                            dc.put_shell_icon(RECYCLE_BIN_CACHE_KEY, &fresh, w, h);
                            let _ = tx.send(AsyncIconResult {
                                key: "__recyclebin__".to_string(),
                                data: Some((fresh, w, h)),
                            });
                        }
                    }
                });

                return Some(texture);
            }
        }

        // Cache miss — extract synchronously.
        if let Ok((data, width, height)) = windows::extract_recycle_bin_icon(IconSize::Jumbo) {
            if let Some(dc) = &self.disk_cache {
                dc.put_shell_icon(RECYCLE_BIN_CACHE_KEY, &data, width, height);
            }
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &data);
            let texture = ctx.load_texture("recycle_bin_icon", image, egui::TextureOptions::LINEAR);
            self.recycle_bin_icon_texture = Some(texture.clone());
            return Some(texture);
        }

        None
    }
}
