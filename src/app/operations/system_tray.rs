use crate::app::state::ImageViewerApp;
use eframe::egui;

fn close_routes_to_tray(minimize_to_tray: bool, explicit_exit: bool) -> bool {
    minimize_to_tray && !explicit_exit
}

impl ImageViewerApp {
    pub(crate) fn request_window_close(&mut self, ctx: &egui::Context) {
        if close_routes_to_tray(self.minimize_to_tray, self.tray_exit_requested)
            && self.hide_to_system_tray(ctx)
        {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub(crate) fn handle_tray_and_close_events(&mut self, ctx: &egui::Context) {
        match crate::infrastructure::windows::system_tray::take_action() {
            Some(crate::infrastructure::windows::system_tray::TrayAction::Open) => {
                if let Some(hwnd) = self.native_hwnd {
                    crate::infrastructure::windows::system_tray::show_window(hwnd);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.request_repaint();
            }
            Some(crate::infrastructure::windows::system_tray::TrayAction::Exit) => {
                self.tray_exit_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => {}
        }

        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && close_routes_to_tray(self.minimize_to_tray, self.tray_exit_requested)
        {
            self.hide_to_system_tray(ctx);
        }
    }

    fn hide_to_system_tray(&mut self, ctx: &egui::Context) -> bool {
        let Some(hwnd) = self.native_hwnd else {
            return false;
        };
        if !crate::infrastructure::windows::system_tray::add_tray_icon(hwnd) {
            log::warn!("[TRAY] Could not create the notification-area icon; closing normally");
            return false;
        }

        self.sync_to_tab();
        self.save_preferences();
        if !self.force_save_preferences() {
            log::warn!("[TRAY] Session could not be flushed before hiding the window");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        crate::infrastructure::windows::system_tray::hide_window(hwnd);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::close_routes_to_tray;

    #[test]
    fn close_paths_hide_to_tray_unless_explicit_exit_was_requested() {
        assert!(!close_routes_to_tray(false, false));
        assert!(close_routes_to_tray(true, false));
        assert!(!close_routes_to_tray(true, true));
    }
}
