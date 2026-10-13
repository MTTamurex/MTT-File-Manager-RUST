//! Cleanup modal: scan progress → review/selection → delete progress → summary.
//!
//! Follows the app's modal skeleton (backdrop area + centered card, ESC to
//! cancel) used by the drag-move confirmation modal. This is the single
//! confirmation point for cleanup deletions — the shell operations run
//! silently, so nothing is deleted without passing through here.
//!
//! The review/selection phase lives in the sibling `cleanup_modal_review`
//! module.

use crate::app::state::ImageViewerApp;
use crate::domain::cleanup::CleanupPhase;
use eframe::egui::{self, Color32, Margin, RichText, Vec2};
use rust_i18n::t;

use crate::ui::theme;

const BACKDROP_ALPHA: u8 = 72;
const MODAL_WIDTH: f32 = 560.0;
const MAX_SHOWN_SKIPPED: usize = 50;

/// Translates a worker skip reason into a localized label.
fn skip_reason_label(reason: &str) -> String {
    if let Some(code) = reason.strip_prefix("shell_error_") {
        return t!("cleanup.skip.shell_error", code = code).to_string();
    }
    match reason {
        "cancelled" => t!("cleanup.skip.cancelled").to_string(),
        "rejected" => t!("cleanup.skip.rejected").to_string(),
        "unavailable" => t!("cleanup.skip.unavailable").to_string(),
        "reparse_point" => t!("cleanup.skip.reparse_point").to_string(),
        "modified_since_scan" => t!("cleanup.skip.modified_since_scan").to_string(),
        other => other.to_string(),
    }
}

/// Backdrop click / ESC behavior per phase: while deleting, only signal a
/// graceful cancel (the worker stops after the current item and the modal
/// shows the aborted summary); otherwise close the modal outright.
fn dismiss_or_cancel(app: &mut ImageViewerApp) {
    if app.cleanup_state.phase == CleanupPhase::Deleting {
        app.cancel_cleanup();
    } else {
        app.close_cleanup_modal();
    }
}

pub fn render_cleanup_modal(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if app.cleanup_state.phase == CleanupPhase::Idle {
        return;
    }

    // Backdrop: blocks interaction outside the modal; click cancels.
    let mut close_from_backdrop = false;
    let screen_rect = ctx.viewport_rect();
    egui::Area::new(egui::Id::from("cleanup_modal_backdrop"))
        .fixed_pos(screen_rect.min)
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.set_min_size(screen_rect.size());
            let backdrop_rect = ui.max_rect();
            let response = ui.interact(
                backdrop_rect,
                ui.id().with("cleanup_modal_backdrop_interact"),
                egui::Sense::click(),
            );
            ui.painter().rect_filled(
                backdrop_rect,
                0.0,
                Color32::from_black_alpha(BACKDROP_ALPHA),
            );
            if response.clicked() {
                close_from_backdrop = true;
            }
        });
    if close_from_backdrop {
        dismiss_or_cancel(app);
        return;
    }
    if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        dismiss_or_cancel(app);
        return;
    }

    match app.cleanup_state.phase.clone() {
        CleanupPhase::Scanning => render_scanning(app, ctx, screen_rect),
        CleanupPhase::Review => {
            super::cleanup_modal_review::render_review(app, ctx, screen_rect)
        }
        CleanupPhase::Deleting => render_deleting(app, ctx, screen_rect),
        CleanupPhase::Completed => render_completed(app, ctx, screen_rect),
        CleanupPhase::Failed(message) => render_failed(app, ctx, screen_rect, &message),
        CleanupPhase::Idle => {}
    }
}

pub(crate) fn modal_frame<R>(
    ctx: &egui::Context,
    screen_rect: egui::Rect,
    estimated_height: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let modal_width = MODAL_WIDTH.min(screen_rect.width() - 32.0);
    let modal_x = (screen_rect.width() - modal_width) / 2.0;
    let modal_y = ((screen_rect.height() - estimated_height) / 2.0).max(24.0);

    egui::Area::new(egui::Id::from("cleanup_modal_card"))
        .fixed_pos(egui::pos2(modal_x, modal_y))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let dark_mode = ui.visuals().dark_mode;
            let bg_color = if dark_mode {
                Color32::from_rgb(50, 50, 50)
            } else {
                Color32::from_rgb(250, 250, 250)
            };
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 24,
                    right: 24,
                    top: 20,
                    bottom: 16,
                })
                .corner_radius(10.0)
                .fill(bg_color)
                .stroke(egui::Stroke::new(
                    1.0,
                    if dark_mode {
                        Color32::from_gray(70)
                    } else {
                        Color32::from_gray(220)
                    },
                ))
                .shadow(egui::epaint::Shadow {
                    spread: 4,
                    blur: 12,
                    color: Color32::from_black_alpha(25),
                    offset: [0, 3],
                })
                .show(ui, |ui| {
                    ui.set_width(modal_width - 48.0);
                    content(ui)
                })
                .inner
        })
        .inner
}

pub(crate) fn title_label(ui: &mut egui::Ui, text: impl Into<String>, dark_mode: bool) {
    ui.label(
        RichText::new(text)
            .size(18.0)
            .strong()
            .color(theme::text_color(dark_mode)),
    );
    ui.add_space(12.0);
}

pub(crate) fn secondary_button(ui: &mut egui::Ui, text: &str, dark_mode: bool) -> bool {
    ui.add_sized(
        Vec2::new(110.0, 34.0),
        egui::Button::new(
            RichText::new(text)
                .size(14.0)
                .color(theme::secondary_text_color(dark_mode)),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, theme::secondary_text_color(dark_mode))),
    )
    .clicked()
}

fn render_scanning(app: &mut ImageViewerApp, ctx: &egui::Context, screen_rect: egui::Rect) {
    let progress_snapshot = app
        .cleanup_state
        .scan_progress
        .lock()
        .map(|progress| (progress.entries_seen, progress.bytes_found))
        .unwrap_or((0, 0));

    modal_frame(ctx, screen_rect, 170.0, |ui| {
        let dark_mode = ui.visuals().dark_mode;
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(18.0));
            ui.add_space(10.0);
            ui.label(
                RichText::new(t!("cleanup.scanning").to_string())
                    .size(17.0)
                    .strong()
                    .color(theme::text_color(dark_mode)),
            );
        });
        ui.add_space(12.0);

        let summary = t!(
            "cleanup.scan_progress",
            count = progress_snapshot.0,
            size = crate::infrastructure::windows::formatting::format_size(progress_snapshot.1)
        )
        .to_string();
        ui.label(
            RichText::new(summary)
                .size(14.0)
                .color(theme::secondary_text_color(dark_mode)),
        );
        ui.add_space(16.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, &t!("cleanup.cancel"), dark_mode) {
                // Cancel during a scan discards the run entirely (partial
                // results are never shown as a review list).
                app.close_cleanup_modal();
            }
        });
    });
}

fn render_deleting(app: &mut ImageViewerApp, ctx: &egui::Context, screen_rect: egui::Rect) {
    let Some(progress) = app.cleanup_state.delete_progress.clone() else {
        return;
    };
    let fraction = if progress.total == 0 {
        1.0
    } else {
        progress.completed as f32 / progress.total as f32
    };

    let mut cancel = false;
    modal_frame(ctx, screen_rect, 200.0, |ui| {
        let dark_mode = ui.visuals().dark_mode;
        title_label(ui, t!("cleanup.deleting"), dark_mode);

        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_height(10.0)
                .show_percentage(),
        );
        ui.add_space(6.0);
        let line = t!(
            "cleanup.delete_progress",
            completed = progress.completed,
            total = progress.total
        )
        .to_string();
        ui.label(
            RichText::new(line)
                .size(13.0)
                .color(theme::text_color(dark_mode)),
        );
        if let Some(name) = progress.current_name {
            ui.label(
                RichText::new(t!("cleanup.current_item", name = name).to_string())
                    .size(12.0)
                    .color(theme::secondary_text_color(dark_mode)),
            );
        }
        if progress.failed_count > 0 {
            ui.label(
                RichText::new(
                    t!("cleanup.failed_count", count = progress.failed_count).to_string(),
                )
                .size(12.0)
                .color(theme::secondary_text_color(dark_mode))
                .italics(),
            );
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, &t!("cleanup.cancel"), dark_mode) {
                cancel = true;
            }
        });
    });
    if cancel {
        app.cancel_cleanup();
    }
}

fn render_completed(app: &mut ImageViewerApp, ctx: &egui::Context, screen_rect: egui::Rect) {
    let Some(summary) = app.cleanup_state.summary.clone() else {
        app.cleanup_state.reset();
        return;
    };

    let mut close = false;
    modal_frame(ctx, screen_rect, 240.0, |ui| {
        let dark_mode = ui.visuals().dark_mode;
        title_label(ui, t!("cleanup.done_title"), dark_mode);

        ui.label(
            RichText::new(
                t!(
                    "cleanup.done_freed",
                    size = crate::infrastructure::windows::formatting::format_size(
                        summary.deleted_bytes
                    )
                )
                .to_string(),
            )
            .size(15.0)
            .strong()
            .color(theme::text_color(dark_mode)),
        );
        ui.label(
            RichText::new(
                t!("cleanup.done_deleted", count = summary.deleted_count).to_string(),
            )
            .size(13.0)
            .color(theme::text_color(dark_mode)),
        );
        if summary.aborted {
            ui.label(
                RichText::new(t!("cleanup.aborted_note").to_string())
                    .size(12.0)
                    .color(theme::secondary_text_color(dark_mode))
                    .italics(),
            );
        }
        if !summary.skipped.is_empty() {
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    t!("cleanup.done_skipped", count = summary.skipped.len()).to_string(),
                )
                .size(13.0)
                .color(theme::secondary_text_color(dark_mode)),
            );
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .id_salt("cleanup_skipped_list")
                .show(ui, |ui| {
                    for (index, (path, reason)) in summary.skipped.iter().enumerate() {
                        if index >= MAX_SHOWN_SKIPPED {
                            ui.label(
                                RichText::new(format!(
                                    "… +{}",
                                    summary.skipped.len() - MAX_SHOWN_SKIPPED
                                ))
                                .size(12.0)
                                .color(theme::secondary_text_color(dark_mode)),
                            );
                            break;
                        }
                        let name = path
                            .rsplit(['\\', '/'])
                            .next()
                            .unwrap_or(path.as_str())
                            .to_string();
                        ui.label(
                            RichText::new(format!("• {name} — {}", skip_reason_label(reason)))
                                .size(12.0)
                                .color(theme::secondary_text_color(dark_mode)),
                        );
                    }
                });
        }
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, &t!("cleanup.close"), dark_mode) {
                close = true;
            }
        });
    });
    if close {
        app.close_cleanup_modal();
    }
}

fn render_failed(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    screen_rect: egui::Rect,
    message: &str,
) {
    let mut close = false;
    modal_frame(ctx, screen_rect, 150.0, |ui| {
        let dark_mode = ui.visuals().dark_mode;
        title_label(ui, t!("cleanup.title"), dark_mode);
        ui.label(
            RichText::new(message)
                .size(14.0)
                .color(theme::text_color(dark_mode)),
        );
        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_button(ui, &t!("cleanup.close"), dark_mode) {
                close = true;
            }
        });
    });
    if close {
        app.close_cleanup_modal();
    }
}
