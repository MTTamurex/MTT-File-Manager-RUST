use super::text_syntax::{build_highlight_job, plain_text_job, preview_extension};
use crate::text_viewer::encoding::decode_text;
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub(super) const MAX_TEXT_PREVIEW_BYTES: usize = 256 * 1024;

pub(super) struct TextContent {
    pub text: Arc<String>,
    pub truncated: bool,
    pub layout: Arc<egui::text::LayoutJob>,
}

pub(super) enum PreviewContentStatus {
    Loading,
    Ready(Arc<TextContent>),
    Failed,
}

type TextResult = Result<Arc<TextContent>, String>;

struct TextRequest {
    path: PathBuf,
    dark_mode: bool,
    highlight: bool,
    reply: Sender<TextResult>,
}

struct TextWorker {
    sender: Sender<TextRequest>,
    queue: Receiver<TextRequest>,
}

#[derive(Clone)]
enum LoadState {
    Loading(Receiver<TextResult>),
    Ready(Arc<TextContent>),
    Failed,
}

#[derive(Clone)]
struct PanelState {
    path: PathBuf,
    modified: u64,
    size: u64,
    dark_mode: bool,
    highlight: bool,
    load: LoadState,
}

static TEXT_WORKER: OnceLock<Result<TextWorker, String>> = OnceLock::new();

fn worker() -> Result<&'static TextWorker, String> {
    TEXT_WORKER
        .get_or_init(|| {
            let (sender, queue) = bounded::<TextRequest>(1);
            let worker_queue = queue.clone();
            std::thread::Builder::new()
                .name("detail-text-preview".into())
                .spawn(move || {
                    while let Ok(request) = worker_queue.recv() {
                        let result = std::panic::catch_unwind(|| {
                            load_text(&request.path, request.dark_mode, request.highlight)
                        })
                        .unwrap_or_else(|_| Err("text preview worker panicked".into()));
                        let _ = request.reply.send(result);
                    }
                })
                .map_err(|error| error.to_string())?;
            Ok(TextWorker { sender, queue })
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn request_text(
    path: &Path,
    dark_mode: bool,
    highlight: bool,
) -> Result<Receiver<TextResult>, String> {
    let (reply, result) = bounded(1);
    let request = TextRequest {
        path: path.to_path_buf(),
        dark_mode,
        highlight,
        reply,
    };
    let worker = worker()?;
    super::background::send_latest(&worker.sender, &worker.queue, request)
        .map_err(|_| "text preview worker stopped".to_string())?;
    Ok(result)
}

pub(super) fn load_preview(
    ctx: &egui::Context,
    path: &Path,
    modified: u64,
    size: u64,
    dark_mode: bool,
    highlight: bool,
) -> PreviewContentStatus {
    let id = egui::Id::new("detail_text_preview_state");
    let status = ctx.data_mut(|data| {
        let mut state = data
            .get_temp::<PanelState>(id)
            .filter(|state| {
                state.path == path
                    && state.modified == modified
                    && state.size == size
                    && state.dark_mode == dark_mode
                    && state.highlight == highlight
            })
            .unwrap_or_else(|| {
                let load = request_text(path, dark_mode, highlight)
                    .map(LoadState::Loading)
                    .unwrap_or(LoadState::Failed);
                PanelState {
                    path: path.to_path_buf(),
                    modified,
                    size,
                    dark_mode,
                    highlight,
                    load,
                }
            });

        let completed = match &state.load {
            LoadState::Loading(receiver) => match receiver.try_recv() {
                Ok(Ok(content)) => Some(LoadState::Ready(content)),
                Ok(Err(_)) | Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    Some(LoadState::Failed)
                }
                Err(crossbeam_channel::TryRecvError::Empty) => None,
            },
            LoadState::Ready(_) | LoadState::Failed => None,
        };
        if let Some(completed) = completed {
            state.load = completed;
        }

        let status = match &state.load {
            LoadState::Loading(_) => PreviewContentStatus::Loading,
            LoadState::Ready(content) => PreviewContentStatus::Ready(Arc::clone(content)),
            LoadState::Failed => PreviewContentStatus::Failed,
        };
        data.insert_temp(id, state);
        status
    });
    if matches!(status, PreviewContentStatus::Loading) {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    status
}

fn load_text(path: &Path, dark_mode: bool, highlight: bool) -> TextResult {
    let mut file = File::open(path).map_err(|_| "text preview read failed".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "text preview metadata unavailable".to_string())?;
    if !metadata.is_file() {
        return Err("text preview requires a regular file".into());
    }
    let total_size = metadata.len();
    let bytes_to_read = total_size.min(MAX_TEXT_PREVIEW_BYTES as u64);
    let mut bytes = Vec::with_capacity(bytes_to_read as usize);
    file.by_ref()
        .take(MAX_TEXT_PREVIEW_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "text preview read failed".to_string())?;

    let current_size = file
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(total_size);
    let truncated = current_size.max(total_size) > bytes.len() as u64;
    let text = decode_text(&bytes, truncated)?.text;
    let text = if highlight {
        text
    } else {
        super::markdown_safety::strip_remote_images(&text)
    };
    let layout = if highlight {
        build_highlight_job(path, &text, dark_mode)
            .unwrap_or_else(|| plain_text_job(&text, dark_mode))
    } else {
        egui::text::LayoutJob::default()
    };
    Ok(Arc::new(TextContent {
        text: Arc::new(text),
        truncated,
        layout: Arc::new(layout),
    }))
}

pub(super) fn is_previewable(path: &Path, ctx: &egui::Context) -> bool {
    let extension = preview_extension(path);
    if matches!(
        path.file_name()
            .and_then(|name| name.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("readme" | "license" | "licence" | "makefile" | "dockerfile" | "changelog")
    ) {
        return true;
    }
    crate::text_viewer::is_text_extension(&extension)
        || super::text_syntax::has_syntax(&extension, ctx)
}

pub(super) fn render_text_preview(ui: &mut egui::Ui, file: &crate::domain::file_entry::FileEntry) {
    render(ui, file, true);
}

pub(super) fn render_content_header(
    ui: &mut egui::Ui,
    file: &crate::domain::file_entry::FileEntry,
    title: String,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::text_viewer::is_text_path(&file.path)
                && ui
                    .small_button(rust_i18n::t!("preview.open_text_viewer").to_string())
                    .clicked()
            {
                crate::text_viewer::open_text_viewer(file.path.clone());
            }
        });
    });
}

pub(super) fn load_markdown(
    ctx: &egui::Context,
    file: &crate::domain::file_entry::FileEntry,
    dark_mode: bool,
) -> PreviewContentStatus {
    load_preview(ctx, &file.path, file.modified, file.size, dark_mode, false)
}

fn render(ui: &mut egui::Ui, file: &crate::domain::file_entry::FileEntry, highlight: bool) {
    let dark_mode = ui.visuals().dark_mode;
    render_content_header(ui, file, rust_i18n::t!("preview.text_content").to_string());

    match load_preview(
        ui.ctx(),
        &file.path,
        file.modified,
        file.size,
        dark_mode,
        highlight,
    ) {
        PreviewContentStatus::Loading => {
            ui.add(egui::Spinner::new());
            ui.label(rust_i18n::t!("preview.content_loading").to_string());
        }
        PreviewContentStatus::Failed => {
            ui.colored_label(
                egui::Color32::from_rgb(232, 17, 35),
                rust_i18n::t!("preview.content_unavailable").to_string(),
            );
        }
        PreviewContentStatus::Ready(content) => {
            render_content(ui, &content, dark_mode, &file.path);
        }
    }
}

fn render_content(ui: &mut egui::Ui, content: &TextContent, dark_mode: bool, path: &Path) {
    let background = if dark_mode {
        egui::Color32::from_rgb(24, 24, 24)
    } else {
        egui::Color32::WHITE
    };
    egui::Frame::NONE
        .fill(background)
        .inner_margin(egui::Margin::same(8))
        .corner_radius(4.0)
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt(("detail_text_content", path))
                .max_height(420.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(Arc::clone(&content.layout))
                            .extend()
                            .selectable(true),
                    );
                });
        });
    if content.truncated {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(rust_i18n::t!("preview.text_partial").to_string())
                .small()
                .color(egui::Color32::GRAY),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_and_utf16_bom_content() {
        assert_eq!(
            decode_text("sample text".as_bytes(), false).unwrap().text,
            "sample text"
        );
        assert_eq!(
            decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0], false)
                .unwrap()
                .text,
            "hi"
        );
        assert_eq!(
            decode_text(&[0xFE, 0xFF, 0, b'h', 0, b'i'], false)
                .unwrap()
                .text,
            "hi"
        );
    }

    #[test]
    fn decodes_legacy_text_without_replacement_characters() {
        let (encoded, _, _) = encoding_rs::GBK.encode("中文 content");
        assert_eq!(decode_text(&encoded, false).unwrap().text, "中文 content");
    }

    #[test]
    fn reads_no_more_than_the_preview_limit() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let text = "preview content\n".repeat(30_000);
        std::fs::write(file.path(), &text).unwrap();
        let content = load_text(file.path(), false, false).unwrap();
        assert!(content.truncated);
        assert!(content.text.len() <= MAX_TEXT_PREVIEW_BYTES);
        assert!(content.text.is_char_boundary(content.text.len()));
    }

    #[test]
    fn rejects_binary_data_in_a_text_preview() {
        assert!(decode_text(b"\0\0\0\0\0\0", false).is_err());
    }

    #[test]
    fn pending_text_load_does_not_reenter_the_context_lock() {
        let ctx = egui::Context::default();
        let (_reply, receiver) = bounded(1);
        ctx.data_mut(|data| {
            data.insert_temp(
                egui::Id::new("detail_text_preview_state"),
                PanelState {
                    path: PathBuf::from("file.txt"),
                    modified: 0,
                    size: 0,
                    dark_mode: false,
                    highlight: true,
                    load: LoadState::Loading(receiver),
                },
            )
        });
        assert!(matches!(
            load_preview(&ctx, Path::new("file.txt"), 0, 0, false, true),
            PreviewContentStatus::Loading
        ));
    }
}
