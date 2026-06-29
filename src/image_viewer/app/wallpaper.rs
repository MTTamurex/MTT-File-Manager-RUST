use super::ViewerStatusMessage;
use eframe::egui;
use std::sync::atomic::Ordering;

/// (token, result) sent from the worker thread back to the UI. The token
/// lets the UI drop results that belong to a previous `start_wallpaper` call.
pub(super) type WallpaperOutcome = (u64, Result<(), String>);

impl super::DedicatedImageViewerApp {
    fn reapply_viewer_theme(&self, ctx: &egui::Context) {
        if self.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        #[cfg(target_os = "windows")]
        if let Some(hwnd) = self.native_hwnd {
            crate::infrastructure::windows::window_corners::apply_dark_title_bar(
                hwnd,
                self.dark_mode,
            );
        }
    }

    pub(super) fn start_wallpaper(&mut self, ctx: &egui::Context) {
        if self.wallpaper_in_progress {
            return;
        }

        let Some(path) = self.current_path().cloned() else {
            return;
        };

        // Bump the shared generation: stale workers check it before applying
        // the wallpaper, and `poll_wallpaper` uses the same token for UI state.
        let token = self
            .wallpaper_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.wallpaper_token = token;
        let wallpaper_generation = self.wallpaper_generation.clone();

        let (tx, rx) = std::sync::mpsc::channel::<WallpaperOutcome>();
        let repaint_ctx = ctx.clone();

        self.wallpaper_rx = Some(rx);
        self.wallpaper_in_progress = true;
        self.status_message = Some(ViewerStatusMessage {
            text: rust_i18n::t!("imageviewer.wallpaper_in_progress").to_string(),
            is_error: false,
        });

        let spawn_result = std::thread::Builder::new()
            .name("image-wallpaper".into())
            .spawn(move || {
                let result =
                    crate::image_viewer::wallpaper::set_as_wallpaper_if_current(&path, || {
                        wallpaper_generation.load(Ordering::Acquire) == token
                    });
                let _ = tx.send((token, result));
                repaint_ctx.request_repaint();
            });

        if let Err(err) = spawn_result {
            self.wallpaper_rx = None;
            self.wallpaper_in_progress = false;
            self.status_message = Some(ViewerStatusMessage {
                text: rust_i18n::t!("imageviewer.wallpaper_error", error = err.to_string())
                    .to_string(),
                is_error: true,
            });
        }
    }

    pub(super) fn poll_wallpaper(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.wallpaper_rx else {
            return;
        };

        match rx.try_recv() {
            Ok((token, Ok(()))) => {
                self.wallpaper_rx = None;
                self.wallpaper_in_progress = false;
                if token != self.wallpaper_token {
                    return;
                }
                self.status_message = Some(ViewerStatusMessage {
                    text: rust_i18n::t!("imageviewer.wallpaper_success").to_string(),
                    is_error: false,
                });
                self.reapply_viewer_theme(ctx);
            }
            Ok((token, Err(err))) => {
                self.wallpaper_rx = None;
                self.wallpaper_in_progress = false;
                if token != self.wallpaper_token {
                    return;
                }
                self.status_message = Some(ViewerStatusMessage {
                    text: rust_i18n::t!("imageviewer.wallpaper_error", error = err).to_string(),
                    is_error: true,
                });
                self.reapply_viewer_theme(ctx);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.wallpaper_rx = None;
                self.wallpaper_in_progress = false;
                self.status_message = Some(ViewerStatusMessage {
                    text: rust_i18n::t!(
                        "imageviewer.wallpaper_error",
                        error = "worker disconnected"
                    )
                    .to_string(),
                    is_error: true,
                });
                self.reapply_viewer_theme(ctx);
            }
        }
    }
}
