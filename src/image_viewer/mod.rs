use eframe::egui;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex};

mod ipc;
mod loader;

use crate::viewer_runtime::{apply_saved_locale, build_viewer_native_options};

const IMAGE_VIEWER_MUTEX_NAME: &str = "Global\\MTTFileManager_ImageViewer_SingleInstance\0";
const MAX_IMAGE_FILE_SIZE: u64 = 512 * 1024 * 1024;
const OPEN_REQUEST_DEBOUNCE: Duration = Duration::from_millis(700);

struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    fn try_acquire() -> Option<Self> {
        let wide: Vec<u16> = IMAGE_VIEWER_MUTEX_NAME.encode_utf16().collect();
        unsafe {
            let handle = CreateMutexW(None, true, PCWSTR(wide.as_ptr())).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                None
            } else {
                Some(Self { handle })
            }
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

fn recent_open_request_state() -> &'static Mutex<Option<(PathBuf, Instant)>> {
    static RECENT_OPEN_REQUEST: OnceLock<Mutex<Option<(PathBuf, Instant)>>> = OnceLock::new();
    RECENT_OPEN_REQUEST.get_or_init(|| Mutex::new(None))
}

fn paths_eq_case_insensitive(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
}

fn should_suppress_duplicate_open(path: &Path) -> bool {
    let Ok(mut state) = recent_open_request_state().lock() else {
        return false;
    };

    let now = Instant::now();
    let suppress = state
        .as_ref()
        .map(|(last_path, last_at)| {
            now.duration_since(*last_at) <= OPEN_REQUEST_DEBOUNCE
                && paths_eq_case_insensitive(last_path, path)
        })
        .unwrap_or(false);

    *state = Some((path.to_path_buf(), now));
    suppress
}

fn validate_image_path(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    if path_str.contains('\0') {
        return Err("Path contains null bytes".into());
    }

    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(format!(
                "Path traversal component '{}' not allowed",
                component.as_os_str().to_string_lossy()
            ));
        }
    }

    if path_str.starts_with("\\\\")
        || path_str.starts_with("//")
        || path_str.starts_with("\\\\?\\UNC\\")
    {
        return Err("Network/UNC paths are not allowed".into());
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !crate::infrastructure::windows::is_image_extension(ext) {
        return Err(format!("Unsupported image extension: '{}'", ext));
    }

    if !path.is_file() {
        return Err(format!("File not found: '{}'", path.display()));
    }

    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_IMAGE_FILE_SIZE {
            return Err(format!(
                "File too large: {:.1} MB (max {} MB)",
                meta.len() as f64 / (1024.0 * 1024.0),
                MAX_IMAGE_FILE_SIZE / (1024 * 1024)
            ));
        }
    }

    Ok(())
}

pub fn open_image_viewer(path: PathBuf) {
    log::info!(
        "[IMAGE-VIEWER] open_image_viewer requested pid={} path='{}'",
        std::process::id(),
        path.display()
    );

    if should_suppress_duplicate_open(&path) {
        log::debug!(
            "[IMAGE-VIEWER] suppressing duplicate open request for '{}'",
            path.display()
        );
        return;
    }

    open_image_viewer_blocking(&path);
}

fn open_image_viewer_blocking(path: &Path) {
    if let Err(e) = validate_image_path(path) {
        log::error!(
            "[IMAGE-VIEWER] path validation failed for '{}': {}",
            path.display(),
            e
        );
        return;
    }

    match ipc::send_open_request(path) {
        Ok(true) => {
            log::info!(
                "[IMAGE-VIEWER] existing instance accepted open request pid={} path='{}'",
                std::process::id(),
                path.display()
            );
            return;
        }
        Ok(false) => {}
        Err(err) => {
            log::warn!(
                "[IMAGE-VIEWER] failed to forward open request to existing instance: {}",
                err
            );
        }
    }

    let exe = match std::env::current_exe() {
        Ok(v) => v,
        Err(err) => {
            log::error!(
                "[IMAGE-VIEWER] failed to locate current executable for spawn: {}",
                err
            );
            return;
        }
    };

    match Command::new(exe).arg("--image-viewer").arg(path).spawn() {
        Ok(child) => {
            log::info!(
                "[IMAGE-VIEWER] spawned standalone viewer parent_pid={} child_pid={} path='{}'",
                std::process::id(),
                child.id(),
                path.display()
            );
        }
        Err(err) => {
            log::error!(
                "[IMAGE-VIEWER] failed to spawn standalone viewer for '{}': {}",
                path.display(),
                err
            );
        }
    }
}

pub fn run_standalone(path: PathBuf) -> eframe::Result<()> {
    log::info!(
        "[IMAGE-VIEWER] run_standalone enter pid={} path='{}'",
        std::process::id(),
        path.display()
    );

    if let Err(e) = validate_image_path(&path) {
        log::error!("[IMAGE-VIEWER] path validation failed in standalone: {}", e);
        return Ok(());
    }

    apply_saved_locale();

    let _guard = match SingleInstanceGuard::try_acquire() {
        Some(g) => g,
        None => {
            match ipc::send_open_request(&path) {
                Ok(true) => {
                    log::info!("[IMAGE-VIEWER] forwarded image to existing viewer instance");
                }
                Ok(false) => {
                    log::warn!("[IMAGE-VIEWER] existing instance unavailable for IPC forward");
                }
                Err(err) => {
                    log::warn!("[IMAGE-VIEWER] failed to forward image to existing viewer: {}", err);
                }
            }
            return Ok(());
        }
    };

    let external_open_rx = ipc::start_open_request_server();
    let title = title_for_path(&path);

    // Pre-read bytes so the texture is ready inside the eframe creator callback.
    let initial_bytes = std::fs::read(&path).ok();

    // Mirror PDF/text viewer startup exactly: hidden viewport + app_id + reveal
    // on the first update frame. PDF/text never flicker with this pattern.
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([1200.0, 850.0])
        .with_visible(false)
        .with_resizable(true)
        .with_decorations(true)
        .with_app_id("mtt-file-manager-image-viewer");

    if let Ok(img) = image::load_from_memory(crate::embedded_assets::APP_ICON_PNG) {
        let resized = img.resize_exact(256, 256, image::imageops::FilterType::CatmullRom);
        let rgba_image = resized.to_rgba8();
        viewport = viewport.with_icon(egui::IconData {
            rgba: rgba_image.into_raw(),
            width: 256,
            height: 256,
        });
    }

    let native_options = build_viewer_native_options(viewport);

    eframe::run_native(
        &rust_i18n::t!("imageviewer.title"),
        native_options,
        Box::new(move |cc| {
            let mut app = SimpleImageViewerApp::new(path.clone(), external_open_rx);
            if let Some(bytes) = initial_bytes {
                app.upload_texture_from_bytes(&cc.egui_ctx, &bytes);
            } else {
                app.last_error = Some(format!("Failed to read image: {}", path.display()));
            }
            Ok(Box::new(app))
        }),
    )
}

struct SimpleImageViewerApp {
    external_open_rx: Receiver<PathBuf>,
    current_path: PathBuf,
    texture: Option<egui::TextureHandle>,
    texture_size: egui::Vec2,
    zoom_factor: f32,
    fit_to_window: bool,
    last_error: Option<String>,
    revealed: bool,
}

impl SimpleImageViewerApp {
    fn new(current_path: PathBuf, external_open_rx: Receiver<PathBuf>) -> Self {
        Self {
            external_open_rx,
            current_path,
            texture: None,
            texture_size: egui::Vec2::ZERO,
            zoom_factor: 1.0,
            fit_to_window: true,
            last_error: None,
            revealed: false,
        }
    }

    fn upload_texture_from_bytes(&mut self, ctx: &egui::Context, bytes: &[u8]) {
        match image::load_from_memory(bytes) {
            Ok(decoded) => {
                let rgba = decoded.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                self.texture_size = egui::vec2(rgba.width() as f32, rgba.height() as f32);
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                self.texture = Some(ctx.load_texture(
                    "image-viewer-current",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                self.last_error = None;
            }
            Err(err) => {
                self.texture = None;
                self.texture_size = egui::Vec2::ZERO;
                self.last_error = Some(format!("Failed to decode image: {}", err));
            }
        }
    }

    fn load_current_image(&mut self, ctx: &egui::Context) {
        match std::fs::read(&self.current_path) {
            Ok(bytes) => {
                self.upload_texture_from_bytes(ctx, &bytes);
                if self.texture.is_some() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title_for_path(
                        &self.current_path,
                    )));
                }
            }
            Err(err) => {
                self.texture = None;
                self.texture_size = egui::Vec2::ZERO;
                self.last_error = Some(format!("Failed to read image: {}", err));
            }
        }
    }

    fn handle_external_open_requests(&mut self, ctx: &egui::Context) {
        let mut latest: Option<PathBuf> = None;
        while let Ok(path) = self.external_open_rx.try_recv() {
            latest = Some(path);
        }

        if let Some(path) = latest {
            if validate_image_path(&path).is_ok() {
                self.current_path = path;
                self.zoom_factor = 1.0;
                self.fit_to_window = true;
                self.load_current_image(ctx);
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let zoom_in = ctx.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals));
        let zoom_out = ctx.input(|i| i.key_pressed(egui::Key::Minus));
        let reset_zoom = ctx.input(|i| i.key_pressed(egui::Key::Num0));
        let toggle_fit = ctx.input(|i| i.key_pressed(egui::Key::F));

        if zoom_in {
            self.fit_to_window = false;
            self.zoom_factor = (self.zoom_factor * 1.1).clamp(0.1, 8.0);
        }
        if zoom_out {
            self.fit_to_window = false;
            self.zoom_factor = (self.zoom_factor / 1.1).clamp(0.1, 8.0);
        }
        if reset_zoom {
            self.fit_to_window = true;
            self.zoom_factor = 1.0;
        }
        if toggle_fit {
            self.fit_to_window = !self.fit_to_window;
        }
    }

    fn render_ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("image_viewer_top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(self.current_path.display().to_string());
                ui.separator();
                ui.label("+/- zoom");
                ui.separator();
                ui.label("0 fit");
                ui.separator();
                ui.label("F toggle fit");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.last_error {
                ui.colored_label(egui::Color32::from_rgb(220, 80, 80), err);
                return;
            }

            let Some(texture) = &self.texture else {
                ui.label("Loading image...");
                return;
            };

            let available = ui.available_size();
            let mut draw_size = self.texture_size;
            if self.fit_to_window {
                let scale = (available.x / self.texture_size.x)
                    .min(available.y / self.texture_size.y)
                    .min(1.0);
                draw_size *= scale;
            } else {
                draw_size *= self.zoom_factor;
            }

            egui::ScrollArea::both().show(ui, |ui| {
                ui.image(egui::load::SizedTexture::new(texture.id(), draw_size));
            });
        });
    }
}

impl eframe::App for SimpleImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_external_open_requests(ctx);
        self.handle_shortcuts(ctx);
        self.render_ui(ctx);

        // Mirror PDF/text viewer reveal pattern: only show the window AFTER the
        // first frame has been laid out, so the OS never sees an empty window.
        if !self.revealed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.revealed = true;
        }
    }
}

fn title_for_path(path: &Path) -> String {
    let title_name = path
        .file_name()
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| rust_i18n::t!("imageviewer.title").to_string());

    rust_i18n::t!("imageviewer.title_with_file", name = title_name).to_string()
}

pub fn decode_full_for_benchmark(path: &Path) -> std::io::Result<(u32, u32, usize)> {
    let frame = loader::decode_full_frame(path)?;
    Ok((frame.width, frame.height, frame.rgba.len()))
}

pub fn decode_preview_for_benchmark(
    path: &Path,
    max_side: u32,
) -> std::io::Result<(u32, u32, usize)> {
    let frame = loader::decode_preview_frame(path, max_side)?;
    Ok((frame.width, frame.height, frame.rgba.len()))
}
