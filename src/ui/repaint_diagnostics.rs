//! Privacy-safe sampling of UI passes, not GPU submissions or presentations.
//! Repaint causes are egui's recorded requests from a pass; they are clues, not
//! exclusive attribution of the event that woke the viewport.
use crate::app::ImageViewerApp;
use crate::infrastructure::diagnostic_logger::{self, field_bool, field_label, field_u64};
use eframe::egui;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_CAUSES_PER_SAMPLE: usize = 12;

mod sampler;
use sampler::{Recorder, WindowState};

thread_local! {
    static RECORDER: RefCell<Recorder> = RefCell::new(Recorder::default());
}

pub(crate) fn record_idle_frame(app: &ImageViewerApp, ctx: &egui::Context) {
    record(app, ctx, true);
}

pub(crate) fn record_idle_logic(app: &ImageViewerApp, ctx: &egui::Context) {
    record(app, ctx, false);
}

fn record(app: &ImageViewerApp, ctx: &egui::Context, ui_pass: bool) {
    let Some(session) = diagnostic_logger::capture_session() else {
        RECORDER.with(|recorder| *recorder.borrow_mut() = Recorder::default());
        return;
    };

    let now = Instant::now();
    let input_received = ctx.input(|input| !input.events.is_empty());
    let causes = if ui_pass {
        ctx.repaint_causes()
    } else {
        Vec::new()
    };
    let viewport_visible = ctx.input(|input| input.viewport().visible().unwrap_or(true));
    let viewport_focused = ctx.input(|input| input.viewport().focused.unwrap_or(false));
    let minimized = app.layout.saved_is_minimized
        || crate::infrastructure::windows::window_subclass::layout_phase()
            == crate::infrastructure::windows::window_subclass::WindowLayoutPhase::Minimized;

    let state = WindowState {
        visible: viewport_visible,
        focused: viewport_focused,
        minimized,
    };
    let report = RECORDER.with(|recorder| {
        recorder.borrow_mut().record(
            session,
            now,
            state,
            input_received,
            ui_pass.then(|| ctx.cumulative_pass_nr()),
            ui_pass && ctx.requested_repaint_last_pass(),
            causes,
        )
    });

    let Some(report) = report else {
        return;
    };
    let duration_ms = report.elapsed.as_millis().max(1).min(u64::MAX as u128) as u64;
    let mut causes = report.causes;
    diagnostic_logger::diag_info(
        "repaint_idle",
        "sample",
        &[
            field_label("measurement", "ui_passes_not_presentations"),
            field_u64("ui_passes", report.ui_passes),
            field_u64(
                "passes_with_zero_previous_repaint_delay",
                report.zero_previous_delay_passes,
            ),
            field_u64(
                "passes_without_zero_previous_repaint_delay",
                report
                    .ui_passes
                    .saturating_sub(report.zero_previous_delay_passes),
            ),
            field_label(
                "cause_measurement",
                "recorded_requests_not_exclusive_wakeups",
            ),
            field_bool("gpu_presentations_observable", false),
            field_u64("logic_ticks", report.logic_ticks),
            field_u64("duration_ms", duration_ms),
            field_u64(
                "ui_passes_per_second_milli",
                report.ui_passes.saturating_mul(1_000_000) / duration_ms,
            ),
            field_u64("causes_overflow", report.causes_overflow),
            field_bool("viewport_visible", viewport_visible),
            field_bool("viewport_focused", viewport_focused),
            field_bool("minimized", minimized),
            field_bool("folder_loading", app.is_loading_folder),
            field_bool("scroll_settling", app.thumbnail_scroll_is_settling()),
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
                "file_operations",
                app.file_operation_state.file_ops_in_progress as u64,
            ),
        ],
    );

    causes.sort_unstable_by_key(|(_, _, count)| std::cmp::Reverse(*count));
    for (source, line, count) in causes.into_iter().take(MAX_CAUSES_PER_SAMPLE) {
        diagnostic_logger::diag_info(
            "repaint_idle",
            "cause",
            &[
                field_label("source", source),
                field_u64("line", line as u64),
                field_u64("observations", count),
            ],
        );
    }
}

fn classify_source(file: &str) -> &'static str {
    let normalized = file.replace('\\', "/");
    let source = normalized
        .rsplit_once("/src/")
        .map(|(_, source)| source)
        .or_else(|| normalized.strip_prefix("src/"))
        .unwrap_or(normalized.as_str());
    match source {
        "ui/app_impl.rs" => "ui_app",
        "ui/clipboard_feedback.rs" | "ui/clipboard_feedback/listener.rs" => "clipboard_feedback",
        "app/state/helpers.rs" => "app_state",
        "app/operations/drive_refresh.rs" => "drive_refresh",
        "app/operations/drive_refresh/worker.rs" => "drive_refresh_worker",
        "app/operations/view_setup.rs" => "drive_list_scan",
        "app/operations/external_open.rs" => "external_open",
        "ui/icon_loader/async_ops.rs" => "icon_loader",
        "app/operations/selection.rs" => "selection",
        "ui/app/input.rs" => "ui_input",
        "ui/video_overlay.rs" => "video_overlay",
        "app/operations/message_handler/thumbnail_uploads.rs" => "thumbnail_uploads",
        "infrastructure/onedrive/google_drive_status.rs" => "cloud_status",
        "workers/global_search_worker.rs" => "global_search_worker",
        "app/update_state.rs" => "app_update",
        "workers/thumbnail/worker.rs" | "workers/thumbnail/worker/request_processing.rs" => {
            "thumbnail_worker"
        }
        "workers/folder_preview_worker.rs" | "workers/folder_preview_worker/cache_first.rs" => {
            "folder_preview_worker"
        }
        "image_viewer/app/mod.rs" => "image_viewer",
        "ui/app/notifications.rs" => "notifications",
        "app/column_state.rs" | "app/column_state/grouping.rs" | "app/column_state/sorting.rs" => {
            "column_state"
        }
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::classify_source;

    #[test]
    fn zero_previous_delay_does_not_prove_an_explicit_immediate_request() {
        let ctx = eframe::egui::Context::default();
        for _ in 0..5 {
            let _ = ctx.run_ui(Default::default(), |_| {});
        }
        let _ = ctx.run_ui(Default::default(), |ui| {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(1));
        });
        let _ = ctx.run_ui(Default::default(), |ui| {
            assert!(ui.ctx().requested_repaint_last_pass());
        });
    }

    #[test]
    fn source_classification_never_returns_a_path() {
        assert_eq!(
            classify_source(r"C:\build\repo\src\workers\thumbnail\worker.rs"),
            "thumbnail_worker"
        );
        assert_eq!(classify_source("src/private_module.rs"), "other");
        assert_eq!(
            classify_source("src/ui/clipboard_feedback.rs"),
            "clipboard_feedback"
        );
        assert_eq!(
            classify_source("C:/build/repo/src/app/operations/drive_refresh.rs"),
            "drive_refresh"
        );
        assert_eq!(
            classify_source("src/app/operations/drive_refresh/worker.rs"),
            "drive_refresh_worker"
        );
        assert_eq!(
            classify_source(r"C:\build\repo\src\ui\clipboard_feedback\listener.rs"),
            "clipboard_feedback"
        );
        assert_eq!(classify_source("src/ui/video_overlay.rs"), "video_overlay");
        for (file, expected) in [
            ("src/app/operations/view_setup.rs", "drive_list_scan"),
            ("src/app/operations/external_open.rs", "external_open"),
            ("src/ui/icon_loader/async_ops.rs", "icon_loader"),
            ("src/app/operations/selection.rs", "selection"),
            ("src/ui/app/input.rs", "ui_input"),
            (
                "src/app/operations/message_handler/thumbnail_uploads.rs",
                "thumbnail_uploads",
            ),
            (
                "src/infrastructure/onedrive/google_drive_status.rs",
                "cloud_status",
            ),
            (
                "src/workers/global_search_worker.rs",
                "global_search_worker",
            ),
        ] {
            assert_eq!(classify_source(file), expected, "{file}");
        }
    }
}
