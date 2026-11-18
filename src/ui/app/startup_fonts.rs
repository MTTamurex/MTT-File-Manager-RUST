use crate::app::ImageViewerApp;
use eframe::egui;
use std::sync::mpsc::TryRecvError;

pub(crate) fn apply_ready_startup_fonts(app: &mut ImageViewerApp, ctx: &egui::Context) -> bool {
    let result = app
        .font_loader_rx
        .as_ref()
        .map(|receiver| receiver.try_recv());
    match result {
        None => true,
        Some(Ok(fonts)) => {
            crate::ui::fonts::selection::install(
                ctx,
                fonts,
                crate::ui::fonts::sizing::FontSizes::load(&app.app_state_db),
            );
            crate::ui::fonts::selection::flush_font_updates(ctx);
            app.font_loader_rx = None;
            true
        }
        Some(Err(TryRecvError::Empty)) => false,
        Some(Err(TryRecvError::Disconnected)) => {
            log::warn!("[FONTS] Initial font loader disconnected; using startup fallback fonts");
            app.font_loader_rx = None;
            true
        }
    }
}
