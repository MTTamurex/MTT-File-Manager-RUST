use crate::domain::file_entry::FileEntry;
use crate::ui::preview_panel::image_preview::render_texture_with_overlay;
use crate::ui::svg_icons::SvgIconManager;
use crossbeam_channel::{bounded, Receiver, Sender};
use eframe::egui;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;

const MAX_SVG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RENDER_SIDE: u32 = 2048;

struct SvgRequest {
    path: PathBuf,
    reply: Sender<Result<egui::ColorImage, String>>,
}

struct SvgWorker {
    sender: Sender<SvgRequest>,
    queue: Receiver<SvgRequest>,
}

static SVG_WORKER: OnceLock<Result<SvgWorker, String>> = OnceLock::new();

fn worker() -> Result<&'static SvgWorker, String> {
    SVG_WORKER
        .get_or_init(|| {
            let (sender, queue) = bounded::<SvgRequest>(1);
            let worker_queue = queue.clone();
            std::thread::Builder::new()
                .name("detail-svg-preview".into())
                .spawn(move || {
                    while let Ok(request) = worker_queue.recv() {
                        let result = std::panic::catch_unwind(|| rasterize(&request.path))
                            .unwrap_or_else(|_| Err("SVG preview worker panicked".into()));
                        let _ = request.reply.send(result);
                    }
                })
                .map_err(|error| error.to_string())?;
            Ok(SvgWorker { sender, queue })
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn request_render(path: PathBuf) -> Result<Receiver<Result<egui::ColorImage, String>>, String> {
    let (reply, result) = bounded(1);
    let request = SvgRequest { path, reply };
    let worker = worker()?;
    super::background::send_latest(&worker.sender, &worker.queue, request)
        .map_err(|_| "SVG preview worker stopped".to_string())?;
    Ok(result)
}

#[derive(Default, Clone)]
struct SvgPanelState {
    path: Option<PathBuf>,
    modified: u64,
    size: u64,
    texture: Option<egui::TextureHandle>,
    in_flight: Option<Receiver<Result<egui::ColorImage, String>>>,
    failed: bool,
}

pub(super) fn render_svg_preview(
    ui: &mut egui::Ui,
    file: &FileEntry,
    svg_manager: &mut SvgIconManager,
) {
    let id = egui::Id::new("detail_svg_preview_state");
    let ctx = ui.ctx().clone();
    let mut state = ctx.data_mut(|data| {
        let mut state: SvgPanelState = data.get_temp(id).unwrap_or_default();
        if state.path.as_ref() != Some(&file.path)
            || state.modified != file.modified
            || state.size != file.size
        {
            state = SvgPanelState {
                path: Some(file.path.clone()),
                modified: file.modified,
                size: file.size,
                ..Default::default()
            };
            match request_render(file.path.clone()) {
                Ok(receiver) => state.in_flight = Some(receiver),
                Err(_) => state.failed = true,
            }
        }

        state
    });

    if let Some(receiver) = state.in_flight.as_ref() {
        match receiver.try_recv() {
            Ok(Ok(image)) => {
                state.texture = Some(ctx.load_texture(
                    "detail_svg_preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                state.in_flight = None;
                state.failed = false;
            }
            Ok(Err(_)) | Err(crossbeam_channel::TryRecvError::Disconnected) => {
                state.in_flight = None;
                state.failed = true;
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
        }
    }

    super::image_preview::render_centered_preview(ui, |ui| {
        ui.label(egui::RichText::new(rust_i18n::t!("preview.svg_content").to_string()).strong());
        if let Some(texture) = &state.texture {
            let _ = render_texture_with_overlay(ui, file, texture, svg_manager);
        } else if state.failed {
            ui.colored_label(
                egui::Color32::from_rgb(232, 17, 35),
                rust_i18n::t!("preview.content_unavailable").to_string(),
            );
        } else {
            ui.add(egui::Spinner::new());
            ui.label(rust_i18n::t!("preview.content_loading").to_string());
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    });

    ui.ctx().data_mut(|data| {
        data.insert_temp(id, state);
    });
}

fn rasterize(path: &std::path::Path) -> Result<egui::ColorImage, String> {
    let file = File::open(path).map_err(|_| "SVG preview read failed".to_string())?;
    let metadata = file
        .metadata()
        .map_err(|_| "SVG preview metadata unavailable".to_string())?;
    if !metadata.is_file() {
        return Err("SVG preview requires a regular file".into());
    }
    let size = metadata.len();
    if size > MAX_SVG_BYTES {
        return Err("SVG file exceeds preview limit".into());
    }

    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_SVG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "SVG preview read failed".to_string())?;
    if bytes.len() as u64 > MAX_SVG_BYTES || bytes.starts_with(&[0x1f, 0x8b]) {
        return Err("SVG input exceeds preview policy".into());
    }
    let options = preview_options();
    let frame =
        crate::image_viewer::loader::decode_svg_preview_bytes(&bytes, MAX_RENDER_SIDE, &options)
            .map_err(|_| "SVG rasterization failed".to_string())?;
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [frame.width as usize, frame.height as usize],
        &frame.rgba,
    ))
}

fn preview_options() -> usvg::Options<'static> {
    let mut options = usvg::Options::default();
    // usvg's default resolver reads arbitrary paths, including UNC shares.
    options.image_href_resolver.resolve_string = Box::new(|_, _| None);
    let default_data = usvg::ImageHrefResolver::default_data_resolver();
    let pixels = std::sync::atomic::AtomicU64::new(0);
    options.image_href_resolver.resolve_data = Box::new(move |mime, data, options| {
        if data.starts_with(&[0x1f, 0x8b]) {
            return None;
        }
        // Bound embedded raster decoding, not only the final SVG surface.
        let reader = image::ImageReader::new(std::io::Cursor::new(data.as_slice()))
            .with_guessed_format()
            .ok()?;
        if reader.format().is_some() {
            let (width, height) = reader.into_dimensions().ok()?;
            let count = u64::from(width) * u64::from(height);
            if count > u64::from(MAX_RENDER_SIDE).pow(2)
                || pixels.fetch_add(count, std::sync::atomic::Ordering::Relaxed) + count
                    > 4 * u64::from(MAX_RENDER_SIDE).pow(2)
            {
                return None;
            }
        }
        default_data(mime, data, options)
    });
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_rasterization_preserves_alpha_and_bounds_dimensions() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="red"/></svg>"#,
        )
        .unwrap();
        let image = rasterize(file.path()).unwrap();
        assert_eq!(image.size, [100, 50]);
    }

    #[test]
    fn rejects_svg_files_above_the_read_limit() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file()
            .set_len(MAX_SVG_BYTES + 1)
            .expect("set generic test file size");
        assert!(rasterize(file.path()).is_err());
    }

    #[test]
    fn preview_resolver_never_reads_external_images() {
        let options = preview_options();
        for target in [
            "image.png",
            "https://example.org/image.png",
            r"\\server\share\image.png",
        ] {
            assert!((options.image_href_resolver.resolve_string)(target, &options).is_none());
        }
    }

    #[test]
    fn oversized_svg_is_scaled_and_transparency_is_preserved() {
        let options = preview_options();
        let frame = crate::image_viewer::loader::decode_svg_preview_bytes(
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="8192" height="4096"><rect width="4096" height="4096" fill="red"/></svg>"#,
            MAX_RENDER_SIDE,
            &options,
        ).unwrap();
        assert_eq!((frame.width, frame.height), (2048, 1024));
        assert_eq!(&frame.rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&frame.rgba[frame.rgba.len() - 4..], &[0, 0, 0, 0]);
    }

    #[test]
    fn completed_svg_upload_does_not_reenter_the_context_lock() {
        let ctx = egui::Context::default();
        let file = FileEntry::from_path(PathBuf::from("drawing.svg"), false);
        let (reply, receiver) = bounded(1);
        reply
            .send(Ok(egui::ColorImage::new([1, 1], vec![egui::Color32::RED])))
            .unwrap();
        let id = egui::Id::new("detail_svg_preview_state");
        ctx.data_mut(|data| {
            data.insert_temp(
                id,
                SvgPanelState {
                    path: Some(file.path.clone()),
                    modified: file.modified,
                    size: file.size,
                    in_flight: Some(receiver),
                    ..Default::default()
                },
            )
        });
        let mut svg_manager = SvgIconManager::new();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            render_svg_preview(ui, &file, &mut svg_manager);
        });
        assert!(ctx.data(|data| data
            .get_temp::<SvgPanelState>(id)
            .unwrap()
            .texture
            .is_some()));
    }
}
