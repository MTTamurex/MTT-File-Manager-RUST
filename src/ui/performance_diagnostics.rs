//! Frame context uses only app state; never probe disk to explain a slow frame.
use crate::app::ImageViewerApp;
use crate::infrastructure::diagnostic_logger::{self, field_bool, field_u64, frame_trace};
use eframe::egui;

pub(super) fn record_context(app: &ImageViewerApp, ctx: &egui::Context) {
    if diagnostic_logger::capture_session().is_none() {
        return;
    }
    frame_trace::context(vec![
        field_bool(
            "viewport_active",
            ctx.input(|i| i.viewport().focused.unwrap_or(false)),
        ),
        field_bool("repaint_requested", ctx.has_requested_repaint()),
        field_bool("scrolling", app.thumbnail_scroll_is_settling()),
        field_u64("pending_thumbnails", app.pending_thumbnails.len() as u64),
        field_u64(
            "queued_thumbnails",
            app.thumbnail_queue.pending_count() as u64,
        ),
        field_u64(
            "pending_uploads",
            app.cache_manager.pending_upload_set.len() as u64,
        ),
        field_u64(
            "folder_previews_loading",
            app.cache_manager.folder_preview_loading.len() as u64,
        ),
        field_u64(
            "folder_sizes_loading",
            app.folder_size_state.loading.len() as u64,
        ),
        field_u64(
            "folder_sizes_batch_loading",
            app.folder_size_state.batch_loading.len() as u64,
        ),
        field_u64(
            "file_operations",
            app.file_operation_state.file_ops_in_progress as u64,
        ),
        field_bool("items_rebuild", app.items_rebuild_in_flight),
        field_u64(
            "diagnostic_write_failures",
            diagnostic_logger::writer_failures(),
        ),
    ]);
}

pub(crate) fn record_gap(
    app: &ImageViewerApp,
    kind: &'static str,
    duration: std::time::Duration,
    repaint_requested: bool,
) {
    if diagnostic_logger::capture_session().is_none() {
        return;
    }
    frame_trace::request_dump(kind);
    diagnostic_logger::diag_warn(
        "frame_trace",
        "execution_gap",
        &[
            diagnostic_logger::field_label("kind", kind),
            diagnostic_logger::field_duration_ms("duration", duration),
            diagnostic_logger::field_duration_ms(
                "previous_ui_duration",
                std::time::Duration::from_secs_f64(
                    (app.last_actual_frame_ms as f64 / 1000.0).max(0.0),
                ),
            ),
            field_bool("repaint_requested", repaint_requested),
            field_bool("scrolling", app.thumbnail_scroll_is_settling()),
            field_u64("pending_thumbnails", app.pending_thumbnails.len() as u64),
            field_u64(
                "pending_uploads",
                app.cache_manager.pending_upload_set.len() as u64,
            ),
        ],
    );
}
