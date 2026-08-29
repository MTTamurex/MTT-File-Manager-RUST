use crate::image_viewer::crop::NormalizedCrop;
use crate::image_viewer::loader::{self, ExportImageFormat};
use eframe::egui;
use rfd::FileDialog;
use rust_i18n::t;
use std::path::{Path, PathBuf};

use super::crop_selection::{drag_origin, normalized_position, paint_crop_overlay, resize_anchor};

pub(super) struct CropState {
    active: bool,
    selection: Option<NormalizedCrop>,
    drag_anchor: Option<[f32; 2]>,
    confirmation: Option<CropSaveRequest>,
    receiver: Option<std::sync::mpsc::Receiver<CropSaveOutcome>>,
    saving: bool,
    source_snapshot: Option<crate::image_viewer::save::FileSnapshot>,
    force_close_armed: bool,
    publication_started: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl CropState {
    pub(super) fn new() -> Self {
        Self {
            active: false,
            selection: None,
            drag_anchor: None,
            confirmation: None,
            receiver: None,
            saving: false,
            source_snapshot: None,
            force_close_armed: false,
            publication_started: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn is_saving(&self) -> bool {
        self.saving
    }

    pub(super) fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    pub(super) fn has_confirmation(&self) -> bool {
        self.confirmation.is_some()
    }

    pub(super) fn cancel_confirmation(&mut self) {
        self.confirmation = None;
    }

    fn reset_edit(&mut self) {
        self.active = false;
        self.selection = None;
        self.drag_anchor = None;
        self.confirmation = None;
        self.source_snapshot = None;
        self.force_close_armed = false;
        self.publication_started
            .store(false, std::sync::atomic::Ordering::Release);
    }

    fn should_cancel_close(&mut self) -> bool {
        if !self.saving {
            return false;
        }
        if self.force_close_armed {
            return self
                .publication_started
                .load(std::sync::atomic::Ordering::Acquire);
        }
        self.force_close_armed = true;
        true
    }
}

struct CropSaveRequest {
    source: PathBuf,
    destination: PathBuf,
    format: ExportImageFormat,
    crop: NormalizedCrop,
    rotation: u16,
    gif_frame_index: Option<usize>,
    replace_existing: bool,
    replaces_source: bool,
    expected_source: crate::image_viewer::save::FileSnapshot,
    expected_destination: Option<crate::image_viewer::save::FileSnapshot>,
}

struct CropSaveOutcome {
    destination: PathBuf,
    replaces_source: bool,
    result: Result<(), String>,
}

impl super::DedicatedImageViewerApp {
    pub(super) fn crop_is_active(&self) -> bool {
        self.crop.is_active()
    }

    pub(super) fn crop_is_saving(&self) -> bool {
        self.crop.is_saving()
    }

    pub(super) fn crop_has_selection(&self) -> bool {
        self.crop.has_selection()
    }

    pub(super) fn crop_confirmation_open(&self) -> bool {
        self.crop.has_confirmation()
    }

    pub(super) fn should_cancel_crop_close(&mut self) -> bool {
        let cancel = self.crop.should_cancel_close();
        if cancel {
            let message = if self.crop.force_close_armed
                && self
                    .crop
                    .publication_started
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                t!("imageviewer.crop_finalizing")
            } else {
                t!("imageviewer.crop_close_again")
            };
            self.set_status(message, true);
        }
        cancel
    }

    pub(super) fn crop_can_overwrite(&self) -> bool {
        self.current_path()
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            .and_then(ExportImageFormat::from_raster_extension)
            .is_some()
    }

    pub(super) fn begin_crop(&mut self) {
        if self.crop.saving
            || !self.has_current_texture()
            || self.startup_sequence_rx.is_some()
            || self.retarget_sequence_rx.is_some()
        {
            return;
        }
        self.crop.active = true;
        self.crop.selection = None;
        self.crop.drag_anchor = None;
        self.crop.confirmation = None;
        self.status_message = None;
        let Some(source) = self.current_path() else {
            self.crop.reset_edit();
            return;
        };
        match crate::image_viewer::save::capture_file_snapshot(source) {
            Ok(snapshot) => self.crop.source_snapshot = Some(snapshot),
            Err(error) => {
                self.crop.reset_edit();
                self.set_status(
                    t!("imageviewer.crop_error", error = error.to_string()),
                    true,
                );
            }
        }
    }

    pub(super) fn cancel_crop(&mut self) {
        if !self.crop.saving {
            self.crop.reset_edit();
        }
    }

    pub(super) fn reset_crop_selection(&mut self) {
        self.crop.selection = None;
        self.crop.drag_anchor = None;
        self.crop.confirmation = None;
    }

    pub(super) fn interact_with_crop(
        &mut self,
        response: &egui::Response,
        image_rect: egui::Rect,
        ui: &egui::Ui,
    ) {
        if !self.crop.active {
            return;
        }

        if self.crop.saving || self.crop.has_confirmation() {
            if let Some(selection) = self.crop.selection {
                paint_crop_overlay(ui.painter(), image_rect, selection);
            }
            return;
        }

        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }

        let primary = egui::PointerButton::Primary;
        if response.drag_started_by(primary) {
            if let Some(position) = response.interact_pointer_pos() {
                let press_position =
                    drag_origin(position, response.total_drag_delta().unwrap_or_default());
                let pointer = normalized_position(press_position, image_rect);
                self.crop.drag_anchor = Some(
                    resize_anchor(self.crop.selection, pointer, image_rect).unwrap_or(pointer),
                );
            }
        }

        if response.dragged_by(primary) {
            if let (Some(anchor), Some(position)) =
                (self.crop.drag_anchor, response.interact_pointer_pos())
            {
                self.crop.selection =
                    NormalizedCrop::new(anchor, normalized_position(position, image_rect));
            }
        }

        if response.drag_stopped_by(primary)
            || !ui.input(|input| input.pointer.button_down(primary))
        {
            self.crop.drag_anchor = None;
        }

        if let Some(selection) = self.crop.selection {
            paint_crop_overlay(ui.painter(), image_rect, selection);
        }
    }

    pub(super) fn request_crop_overwrite(&mut self) {
        let Some(format) = self
            .current_path()
            .and_then(|path| path.extension())
            .and_then(|extension| extension.to_str())
            .and_then(ExportImageFormat::from_raster_extension)
        else {
            return;
        };
        match self.build_crop_request(self.current_path().cloned(), format) {
            Ok(request) => self.crop.confirmation = Some(request),
            Err(error) => self.set_status(t!("imageviewer.crop_error", error = error), true),
        }
    }

    pub(super) fn request_crop_save_as(&mut self, format: ExportImageFormat, ctx: &egui::Context) {
        let Some(source) = self.current_path().cloned() else {
            return;
        };
        let stem = source
            .file_stem()
            .map(|name| name.to_string_lossy().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "image".to_string());
        let mut dialog = FileDialog::new()
            .add_filter(format.filter_label(), &[format.extension()])
            .set_file_name(format!("{stem}_cropped.{}", format.extension()));
        if let Some(parent) = source.parent() {
            dialog = dialog.set_directory(parent);
        }
        let Some(selected_path) = dialog.save_file() else {
            return;
        };
        let destination = loader::normalize_export_path(&selected_path, format);

        if let Err(error) = validate_crop_destination(&destination) {
            self.set_status(t!("imageviewer.crop_error", error = error), true);
            return;
        }

        let request = match self.build_crop_request(Some(destination), format) {
            Ok(request) => request,
            Err(error) => {
                self.set_status(t!("imageviewer.crop_error", error = error), true);
                return;
            }
        };
        if request.replace_existing {
            self.crop.confirmation = Some(request);
        } else {
            self.start_crop_worker(request, ctx);
        }
    }

    fn build_crop_request(
        &self,
        destination: Option<PathBuf>,
        format: ExportImageFormat,
    ) -> Result<CropSaveRequest, String> {
        let source = self
            .current_path()
            .cloned()
            .ok_or_else(|| t!("imageviewer.crop_no_image").to_string())?;
        let destination =
            destination.ok_or_else(|| t!("imageviewer.crop_no_destination").to_string())?;
        let crop = self
            .crop
            .selection
            .ok_or_else(|| t!("imageviewer.crop_no_selection").to_string())?;
        let expected_source = self
            .crop
            .source_snapshot
            .ok_or_else(|| t!("imageviewer.crop_source_unavailable").to_string())?;
        let replaces_source = super::paths_eq_case_insensitive(&source, &destination);
        let expected_destination = if replaces_source {
            Some(expected_source)
        } else if destination.exists() {
            Some(
                crate::image_viewer::save::capture_file_snapshot(&destination)
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        Ok(CropSaveRequest {
            source,
            destination,
            format,
            crop,
            rotation: self.rotation,
            gif_frame_index: self
                .gif_animation
                .as_ref()
                .map(|animation| animation.current_frame),
            replace_existing: expected_destination.is_some(),
            replaces_source,
            expected_source,
            expected_destination,
        })
    }

    pub(super) fn render_crop_confirmation(&mut self, ctx: &egui::Context) {
        if !self.crop.has_confirmation() {
            return;
        }

        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(t!("imageviewer.crop_confirm_title").to_string())
            .id(egui::Id::new("image_viewer_crop_confirmation"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(t!("imageviewer.crop_confirm_message"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("imageviewer.cancel").to_string()).clicked() {
                        cancel = true;
                    }
                    if ui
                        .button(t!("imageviewer.crop_confirm_save").to_string())
                        .clicked()
                    {
                        confirm = true;
                    }
                });
            });

        if cancel {
            self.crop.confirmation = None;
        } else if confirm {
            if let Some(request) = self.crop.confirmation.take() {
                self.start_crop_worker(request, ctx);
            }
        }
    }

    fn start_crop_worker(&mut self, request: CropSaveRequest, ctx: &egui::Context) {
        if self.crop.saving {
            return;
        }

        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint_ctx = ctx.clone();
        self.crop.receiver = Some(receiver);
        self.crop.saving = true;
        self.crop
            .publication_started
            .store(false, std::sync::atomic::Ordering::Release);
        let publication_started = self.crop.publication_started.clone();
        self.set_status(t!("imageviewer.crop_saving"), false);

        let spawn_result = std::thread::Builder::new()
            .name("image-crop-save".into())
            .spawn(move || {
                let destination = request.destination.clone();
                let replaces_source = request.replaces_source;
                let result = (|| {
                    let source_guard = crate::image_viewer::save::guard_unchanged_source(
                        &request.source,
                        request.expected_source,
                    )
                    .map_err(localized_io_error)?;
                    #[cfg(target_os = "windows")]
                    let frame = super::actions::prepare_displayed_frame(
                        &request.source,
                        request.gif_frame_index,
                    )?;
                    #[cfg(not(target_os = "windows"))]
                    let frame = {
                        let bytes = source_guard.read_all().map_err(localized_io_error)?;
                        super::actions::prepare_displayed_frame_from_memory(
                            &request.source,
                            request.gif_frame_index,
                            &bytes,
                        )?
                    };
                    crate::image_viewer::save::ensure_file_unchanged(
                        &request.source,
                        request.expected_source,
                    )
                    .map_err(localized_io_error)?;
                    let frame = crate::image_viewer::crop::crop_displayed_frame(
                        frame,
                        request.crop,
                        request.rotation,
                    )?;
                    drop(source_guard);
                    publication_started.store(true, std::sync::atomic::Ordering::Release);
                    let result = crate::image_viewer::save::save_frame_atomically(
                        frame,
                        request.format,
                        &request.destination,
                        request.replace_existing,
                        request.expected_destination,
                    );
                    publication_started.store(false, std::sync::atomic::Ordering::Release);
                    result.map_err(localized_io_error)
                })();
                let _ = sender.send(CropSaveOutcome {
                    destination,
                    replaces_source,
                    result,
                });
                repaint_ctx.request_repaint();
            });

        if let Err(error) = spawn_result {
            self.crop.receiver = None;
            self.crop.saving = false;
            self.set_status(
                t!("imageviewer.crop_error", error = error.to_string()),
                true,
            );
        }
    }

    pub(super) fn poll_crop_save(&mut self, ctx: &egui::Context) {
        let result = self
            .crop
            .receiver
            .as_ref()
            .map(|receiver| receiver.try_recv());
        match result {
            Some(Ok(outcome)) => {
                self.crop.receiver = None;
                self.crop.saving = false;
                self.crop.force_close_armed = false;
                self.crop
                    .publication_started
                    .store(false, std::sync::atomic::Ordering::Release);
                match outcome.result {
                    Ok(()) => {
                        crate::image_viewer::thumbnail_cache::invalidate(&outcome.destination);
                        self.crop.reset_edit();
                        let name = file_name(&outcome.destination);
                        if outcome.replaces_source {
                            self.sequence.current_index = self.current_index;
                            self.reset_after_sequence_change(ctx);
                        } else {
                            if let Err(error) =
                                crate::image_viewer::validate_image_path(&outcome.destination)
                            {
                                self.set_status(
                                    t!(
                                        "imageviewer.crop_saved_open_error",
                                        name = name,
                                        error = error
                                    ),
                                    true,
                                );
                                return;
                            }
                            self.open_requested_path(outcome.destination, ctx);
                        }
                        self.set_status(t!("imageviewer.crop_success", name = name), false);
                    }
                    Err(error) => {
                        self.set_status(t!("imageviewer.crop_error", error = error), true);
                    }
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.crop.receiver = None;
                self.crop.saving = false;
                self.crop.force_close_armed = false;
                self.crop
                    .publication_started
                    .store(false, std::sync::atomic::Ordering::Release);
                self.set_status(
                    t!(
                        "imageviewer.crop_error",
                        error = t!("imageviewer.crop_worker_disconnected")
                    ),
                    true,
                );
            }
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
        }
    }
}

fn validate_crop_destination(path: &Path) -> Result<(), String> {
    let value = path.to_string_lossy();
    if value.contains('\0') {
        return Err(t!("imageviewer.crop_invalid_null").to_string());
    }
    if value.starts_with("\\\\") || value.starts_with("//") || value.starts_with("\\\\?\\UNC\\") {
        return Err(t!("imageviewer.crop_network_path").to_string());
    }
    let parent = path
        .parent()
        .ok_or_else(|| t!("imageviewer.crop_no_parent").to_string())?;
    if !parent.is_dir() {
        return Err(t!("imageviewer.crop_directory_missing").to_string());
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn localized_io_error(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::InvalidData => t!("imageviewer.crop_file_changed").to_string(),
        std::io::ErrorKind::AlreadyExists => t!("imageviewer.crop_destination_created").to_string(),
        std::io::ErrorKind::InvalidInput => t!("imageviewer.crop_invalid_state").to_string(),
        _ => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::CropState;

    #[test]
    fn saving_requires_a_second_close_request_to_force_exit() {
        let mut state = CropState::new();
        state.saving = true;

        assert!(state.should_cancel_close());
        assert!(!state.should_cancel_close());
    }

    #[test]
    fn close_remains_blocked_during_atomic_publication() {
        let mut state = CropState::new();
        state.saving = true;
        assert!(state.should_cancel_close());
        state
            .publication_started
            .store(true, std::sync::atomic::Ordering::Release);

        assert!(state.should_cancel_close());
    }
}
