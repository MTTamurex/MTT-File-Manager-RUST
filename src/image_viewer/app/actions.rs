use crate::image_viewer::loader;
use crate::infrastructure::windows_clipboard::ImageClipboardWriteResult;
use eframe::egui;
use rust_i18n::t;
use std::path::{Path, PathBuf};

pub(in crate::image_viewer) struct CopyImageOutcome {
    path: PathBuf,
    result: Result<ImageClipboardWriteResult, String>,
}

pub(in crate::image_viewer) struct DeleteImageOutcome {
    path: PathBuf,
    deleted: bool,
}

impl super::DedicatedImageViewerApp {
    pub(super) fn start_copy_current_image(&mut self, ctx: &egui::Context) {
        if self.copy_in_progress || self.delete_in_progress || !self.has_current_texture() {
            return;
        }
        let Some(path) = self.current_path().cloned() else {
            return;
        };
        let Some(owner) = self.native_hwnd else {
            self.set_status(
                t!("imageviewer.copy_error", error = "window unavailable"),
                true,
            );
            return;
        };
        let Some(clipboard_sequence) =
            crate::infrastructure::windows_clipboard::clipboard_sequence_number()
        else {
            self.set_status(
                t!("imageviewer.copy_error", error = "clipboard unavailable"),
                true,
            );
            return;
        };

        let rotation = self.rotation;
        let gif_frame_index = self
            .gif_animation
            .as_ref()
            .map(|animation| animation.current_frame);
        let owner_value = owner.0 as isize;
        let bitmap_within_budget = self.image_resolution.is_some_and(|(width, height)| {
            (width as usize)
                .checked_mul(height as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                .is_some_and(|bytes| bytes <= MAX_COPY_RGBA_BYTES)
        });
        let (tx, rx) = std::sync::mpsc::channel();
        self.copy_rx = Some(rx);
        self.copy_in_progress = true;
        self.set_status(t!("imageviewer.copy_in_progress"), false);

        let repaint_ctx = ctx.clone();
        let spawn_result = std::thread::Builder::new()
            .name("image-copy".into())
            .spawn(move || {
                let owner = windows::Win32::Foundation::HWND(owner_value as *mut _);
                let bitmap_result = if bitmap_within_budget {
                    prepare_displayed_frame(&path, gif_frame_index)
                        .and_then(|frame| loader::rotate_frame(frame, rotation))
                        .and_then(|frame| {
                            crate::infrastructure::windows_clipboard::copy_image_file_and_bitmap_to_clipboard(
                                &path,
                                frame.width,
                                frame.height,
                                &frame.rgba,
                                owner,
                                clipboard_sequence,
                            )
                        })
                } else {
                    Err("Image exceeds the bitmap clipboard memory budget".to_string())
                };

                let result = bitmap_result.or_else(|bitmap_error| {
                    log::warn!(
                        "[IMAGE-VIEWER] Bitmap clipboard copy failed for '{}'; preserving file copy: {}",
                        path.display(),
                        bitmap_error
                    );
                    crate::infrastructure::windows_clipboard::copy_files_to_clipboard_if_unchanged(
                        std::slice::from_ref(&path),
                        owner,
                        clipboard_sequence,
                    )
                    .map(|result| {
                        result.map_or(
                            ImageClipboardWriteResult::Superseded,
                            |_| ImageClipboardWriteResult::FilesOnly,
                        )
                    })
                    .map_err(|file_error| {
                        format!(
                            "Bitmap copy failed: {bitmap_error}; file copy failed: {file_error}"
                        )
                    })
                });
                let _ = tx.send(CopyImageOutcome { path, result });
                repaint_ctx.request_repaint();
            });

        if let Err(error) = spawn_result {
            self.copy_rx = None;
            self.copy_in_progress = false;
            self.set_status(
                t!("imageviewer.copy_error", error = error.to_string()),
                true,
            );
        }
    }

    pub(super) fn poll_copy_current_image(&mut self) {
        let result = self.copy_rx.as_ref().map(|receiver| receiver.try_recv());
        match result {
            Some(Ok(outcome)) => {
                self.copy_rx = None;
                self.copy_in_progress = false;
                let name = file_name(&outcome.path);
                match outcome.result {
                    Ok(ImageClipboardWriteResult::Complete) => {
                        self.set_status(t!("imageviewer.copy_success", name = name), false);
                    }
                    Ok(ImageClipboardWriteResult::FilesOnly) => {
                        self.set_status(t!("imageviewer.copy_files_only", name = name), true);
                    }
                    Ok(ImageClipboardWriteResult::Superseded) => {
                        self.set_status(t!("imageviewer.copy_superseded"), false);
                    }
                    Err(error) => {
                        self.set_status(t!("imageviewer.copy_error", error = error), true);
                    }
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.copy_rx = None;
                self.copy_in_progress = false;
                self.set_status(t!("imageviewer.copy_error", error = "worker stopped"), true);
            }
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
        }
    }

    pub(super) fn start_delete_current_image(&mut self, ctx: &egui::Context) {
        if self.delete_in_progress
            || self.copy_in_progress
            || !self.has_current_texture()
            || self.startup_sequence_rx.is_some()
        {
            return;
        }
        let Some(path) = self.current_path().cloned() else {
            return;
        };

        let (tx, rx) = std::sync::mpsc::channel();
        self.delete_rx = Some(rx);
        self.delete_in_progress = true;
        self.set_status(t!("imageviewer.delete_in_progress"), false);

        let repaint_ctx = ctx.clone();
        let spawn_result = std::thread::Builder::new()
            .name("image-delete".into())
            .spawn(move || {
                let _com = crate::infrastructure::windows::com_scope::ComScope::sta();
                let owner =
                    crate::infrastructure::windows::shell_operations::create_shell_op_proxy_window(
                    )
                    .unwrap_or_default();
                let deleted =
                    crate::infrastructure::windows::shell_operations::delete_items_with_shell(
                        std::slice::from_ref(&path),
                        owner,
                    );
                let _ = tx.send(DeleteImageOutcome { path, deleted });
                repaint_ctx.request_repaint();
            });

        if let Err(error) = spawn_result {
            self.delete_rx = None;
            self.delete_in_progress = false;
            self.set_status(
                t!("imageviewer.delete_error", error = error.to_string()),
                true,
            );
        }
    }

    pub(super) fn poll_delete_current_image(&mut self, ctx: &egui::Context) {
        let result = self.delete_rx.as_ref().map(|receiver| receiver.try_recv());
        match result {
            Some(Ok(outcome)) => {
                self.delete_rx = None;
                self.delete_in_progress = false;
                if outcome.deleted {
                    let name = file_name(&outcome.path);
                    self.apply_deleted_path(&outcome.path, ctx);
                    self.set_status(t!("imageviewer.delete_success", name = name), false);
                } else {
                    self.set_status(t!("imageviewer.delete_cancelled"), false);
                }
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.delete_rx = None;
                self.delete_in_progress = false;
                self.set_status(
                    t!("imageviewer.delete_error", error = "worker stopped"),
                    true,
                );
            }
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) | None => {}
        }
    }

    fn apply_deleted_path(&mut self, path: &Path, ctx: &egui::Context) {
        let selected_path = self.current_path().cloned();
        let selected_was_deleted = selected_path
            .as_ref()
            .is_some_and(|selected| super::paths_eq_case_insensitive(selected, path));
        if !self.sequence.remove_path(path) {
            return;
        }

        if !selected_was_deleted {
            if let Some(selected_path) = selected_path {
                if let Some(index) = self
                    .sequence
                    .entries
                    .iter()
                    .position(|entry| super::paths_eq_case_insensitive(entry, &selected_path))
                {
                    self.sequence.current_index = index;
                }
            }
        }
        self.reset_after_sequence_change(ctx);
    }

    pub(super) fn reset_after_sequence_change(&mut self, ctx: &egui::Context) {
        self.current_index = self.sequence.current_index;
        self.cache = crate::image_viewer::cache::WindowCache::new(super::DEFAULT_CACHE_RADIUS);
        self.prefetch = crate::image_viewer::cache::PrefetchEngine::new(
            self.worker_count,
            super::DEFAULT_CACHE_RADIUS,
        );
        if self.repaint_ctx_set {
            self.prefetch.set_repaint_ctx(ctx.clone());
        }
        self.prefetch.set_center(self.current_index);
        self.requested_jobs.clear();
        self.texture = None;
        self.texture_index = None;
        self.image_resolution = None;
        self.last_error = None;
        self.zoom_factor = 1.0;
        self.zoom_percent_display = 100.0;
        self.rotation = 0;
        self.gif_animation = None;
        self.gif_loaded_index = None;
        self.gif_decode_rx = None;
        self.gif_upload_queue = None;
        self.filmstrip.reset();
        self.schedule_window_requests();
        ctx.request_repaint();
    }

    pub(super) fn set_status(&mut self, text: impl Into<String>, is_error: bool) {
        self.status_message = Some(super::ViewerStatusMessage {
            text: text.into(),
            is_error,
        });
    }
}

const MAX_COPY_RGBA_BYTES: usize = 128 * 1024 * 1024;

pub(super) fn prepare_displayed_frame(
    path: &Path,
    gif_frame_index: Option<usize>,
) -> Result<loader::DecodedFrame, String> {
    if let Some(frame_index) = gif_frame_index {
        let frames = loader::decode_gif_frames(path).map_err(|error| error.to_string())?;
        return frames
            .into_iter()
            .nth(frame_index)
            .map(|frame| frame.frame)
            .ok_or_else(|| "Current GIF frame is unavailable".to_string());
    }
    loader::decode_export_frame(path).map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
pub(super) fn prepare_displayed_frame_from_memory(
    path: &Path,
    gif_frame_index: Option<usize>,
    bytes: &[u8],
) -> Result<loader::DecodedFrame, String> {
    if let Some(frame_index) = gif_frame_index {
        let frames =
            loader::decode_gif_frames_from_memory(bytes).map_err(|error| error.to_string())?;
        return frames
            .into_iter()
            .nth(frame_index)
            .map(|frame| frame.frame)
            .ok_or_else(|| "Current GIF frame is unavailable".to_string());
    }
    loader::decode_export_frame_from_memory(path, bytes).map_err(|error| error.to_string())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}
