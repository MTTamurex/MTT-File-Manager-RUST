//! Modal UI; filesystem traversal and rendering live in the application layer.
mod columns;
mod controls;
mod footer;
mod sorting;
#[cfg(test)]
mod tests;
use crate::{
    app::{folder_export_state::FolderExportState, ImageViewerApp},
    application::folder_export::{
        worker::{self, Destination},
        *,
    },
};
use eframe::egui;
use rust_i18n::t;
use std::sync::atomic::Ordering;

fn error_text(error: ExportError) -> String {
    let key = match error {
        ExportError::Cancelled => "export_listing.cancelled",
        ExportError::Read => "export_listing.read_error",
        ExportError::Write => "export_listing.write_error",
        ExportError::TooLarge => "export_listing.too_large",
        ExportError::Exists => "export_listing.exists",
        ExportError::Worker => "export_listing.worker_error",
    };
    t!(key).to_string()
}

fn poll(state: &mut FolderExportState) {
    let Some(job) = state.job.as_ref() else {
        return;
    };
    let result = match job.result.try_recv() {
        Ok(result) => result,
        Err(crossbeam_channel::TryRecvError::Empty) => return,
        Err(crossbeam_channel::TryRecvError::Disconnected) => Err(ExportError::Worker),
    };
    let cancelled = job.progress.cancelled.load(Ordering::Relaxed);
    // File completion wins a cancellation that arrived after the atomic commit.
    state.status = Some(match result {
        Ok(result) => {
            if let Some(text) = result.clipboard {
                if cancelled {
                    error_text(ExportError::Cancelled)
                } else if clipboard_win::set_clipboard_string(&text).is_err() {
                    t!("export_listing.clipboard_error").to_string()
                } else {
                    t!(
                        "export_listing.completed",
                        count = result.count,
                        skipped = result.skipped
                    )
                    .to_string()
                }
            } else {
                t!(
                    "export_listing.completed",
                    count = result.count,
                    skipped = result.skipped
                )
                .to_string()
            }
        }
        Err(error) => error_text(error),
    });
    state.job = None;
}

fn start(state: &mut FolderExportState, ctx: &egui::Context) {
    let destination = if state.clipboard {
        Destination::Clipboard
    } else {
        let extension = state.options.format.extension();
        let Some(path) = rfd::FileDialog::new()
            .set_title(t!("export_listing.title"))
            .set_directory(&state.options.root)
            .set_file_name(format!("listing.{extension}"))
            .add_filter(extension.to_uppercase(), &[extension])
            .save_file()
        else {
            return;
        };
        let path = if path.extension().is_none() {
            path.with_extension(extension)
        } else {
            path
        };
        let overwrite = path.exists();
        if overwrite
            && rfd::MessageDialog::new()
                .set_title(t!("export_listing.title"))
                .set_description(t!("export_listing.overwrite"))
                .set_level(rfd::MessageLevel::Warning)
                .set_buttons(rfd::MessageButtons::YesNo)
                .show()
                != rfd::MessageDialogResult::Yes
        {
            return;
        }
        Destination::File { path, overwrite }
    };
    state.options.selection = state.selected_only.then(|| state.selected_paths.clone());
    state.status = None;
    match worker::start(state.options.clone(), destination, ctx.clone()) {
        Ok(job) => state.job = Some(job),
        Err(error) => state.status = Some(error_text(error)),
    }
}

pub(crate) fn render(app: &mut ImageViewerApp, ctx: &egui::Context) {
    let Some(mut state) = app.folder_export.take() else {
        return;
    };
    poll(&mut state);
    let mut close = false;
    egui::Modal::new(egui::Id::new("folder_listing_export"))
        .frame(
            egui::Frame::window(&ctx.global_style())
                .inner_margin(16.0)
                .corner_radius(12.0),
        )
        .show(ctx, |ui| {
            ui.set_width(680.0_f32.min(ctx.viewport_rect().width() - 48.0).max(240.0));
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
            ui.label(
                egui::RichText::new(t!("export_listing.title"))
                    .size(18.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new(t!("export_listing.description"))
                    .color(crate::ui::theme::secondary_text_color(ui.ctx())),
            );
            egui::Frame::group(ui.style())
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        egui::RichText::new(state.options.root.to_string_lossy())
                            .color(crate::ui::theme::secondary_text_color(ui.ctx())),
                    );
                });
            ui.add_space(4.0);
            let busy = state.job.is_some();
            egui::ScrollArea::vertical()
                .id_salt("export_body")
                .max_height((ctx.viewport_rect().height() - 240.0).clamp(100.0, 440.0))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| controls::render(ui, &mut state));
                });
            if let Some(job) = &state.job {
                ui.horizontal(|ui| {
                    ui.spinner();
                    if job.progress.calculating_sizes.load(Ordering::Relaxed) {
                        ui.label(t!("export_listing.calculating_sizes"));
                    } else {
                        ui.label(t!(
                            "export_listing.progress",
                            count = job.progress.items.load(Ordering::Relaxed)
                        ));
                    }
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            if let Some(status) = &state.status {
                ui.label(status);
            }
            ui.separator();
            let valid = !state.selected_only || !state.selected_paths.is_empty();
            match footer::render(ui, busy, valid).inner {
                footer::Action::Export => {
                    state.options.folder_sizes = app.folder_size_state.export_size_snapshot();
                    start(&mut state, ctx);
                }
                footer::Action::Cancel => {
                    if let Some(job) = &state.job {
                        job.progress.cancelled.store(true, Ordering::Relaxed);
                    }
                }
                footer::Action::Close => close = true,
                footer::Action::None => {}
            }
        });
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        if let Some(job) = &state.job {
            job.progress.cancelled.store(true, Ordering::Relaxed);
        } else {
            close = true;
        }
    }
    if !close {
        app.folder_export = Some(state);
    }
}
