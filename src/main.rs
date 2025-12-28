use eframe::egui;
use lru::LruCache;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::env;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use walkdir::WalkDir;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Storage::FileSystem::*,
    Win32::System::Com::*,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
};

// Caminho padrão
const PATH_PADRAO: &str = "C:\\";

// LRU cache
const CACHE_SIZE: usize = 500;
const MAX_CONCURRENT_LOADS: usize = 50;

// Tipo de item
#[derive(Debug, Clone)]
enum FileSystemItem {
    Directory(PathBuf),
    File(PathBuf),
}

impl FileSystemItem {
    fn path(&self) -> &Path {
        match self {
            FileSystemItem::Directory(p) => p,
            FileSystemItem::File(p) => p,
        }
    }
    
    fn is_directory(&self) -> bool {
        matches!(self, FileSystemItem::Directory(_))
    }
}

// Dados de thumbnail
struct ThumbnailData {
    path: PathBuf,
    image_data: Vec<u8>,
    width: u32,
    height: u32,
}

// Aplicação principal
struct ImageViewerApp {
    current_path: String,
    image_sender: Sender<ThumbnailData>,
    image_receiver: Receiver<ThumbnailData>,
    
    // File system
    items: Vec<FileSystemItem>,
    texture_cache: LruCache<PathBuf, egui::TextureHandle>,
    loading_set: HashSet<PathBuf>,
    
    // UI state
    disks: Vec<String>,
    thumbnail_size: f32,        // Zoom: 64-512
    selected_item: Option<usize>,
    
    loading: bool,
    total_items: usize,
}

impl Default for ImageViewerApp {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let disks = get_all_drives();
        
        let mut app = Self {
            current_path: PATH_PADRAO.to_string(),
            image_sender: sender,
            image_receiver: receiver,
            items: Vec::new(),
            texture_cache: LruCache::new(NonZeroUsize::new(CACHE_SIZE).unwrap()),
            loading_set: HashSet::new(),
            disks,
            thumbnail_size: 128.0,  // Default zoom
            selected_item: None,
            loading: false,
            total_items: 0,
        };
        
        app.load_folder();
        app
    }
}

// Enumera drives
fn get_all_drives() -> Vec<String> {
    unsafe {
        let mut buffer = vec![0u16; 256];
        let len = GetLogicalDriveStringsW(Some(&mut buffer));
        
        if len == 0 {
            return Vec::new();
        }
        
        String::from_utf16_lossy(&buffer[..len as usize])
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
}

// Extrai thumbnail
fn extract_windows_thumbnail(path: &PathBuf) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let shell_item: IShellItem = SHCreateItemFromParsingName(
            PCWSTR(path_wide.as_ptr()),
            None,
        )?;
        
        let image_factory: IShellItemImageFactory = shell_item.cast()?;
        
        let size = SIZE {
            cx: 256,
            cy: 256,
        };
        let hbitmap: HBITMAP = image_factory.GetImage(size, SIIGBF_THUMBNAILONLY)?;
        
        let (rgba_data, width, height) = hbitmap_to_rgba(hbitmap)?;
        
        let _ = DeleteObject(hbitmap);
        
        Ok((rgba_data, width, height))
    }
}

fn hbitmap_to_rgba(hbitmap: HBITMAP) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(
            hbitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        
        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.abs() as usize;
        
        let mut buffer = vec![0u8; width * height * 4];
        
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        
        let hdc = GetDC(None);
        GetDIBits(
            hdc,
            hbitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);
        
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

fn create_error_placeholder() -> (Vec<u8>, u32, u32) {
    let size = 256;
    let mut buffer = vec![0u8; size * size * 4];
    
    for (i, pixel) in buffer.chunks_exact_mut(4).enumerate() {
        let x = i % size;
        let y = i / size;
        let intensity = ((x + y) as f32 / (size * 2) as f32 * 100.0) as u8 + 100;
        pixel[0] = intensity;
        pixel[1] = intensity;
        pixel[2] = intensity;
        pixel[3] = 255;
    }
    
    (buffer, 256, 256)
}

fn open_with_shell(path: &Path) {
    unsafe {
        let path_str = path.to_string_lossy().to_string();
        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
        
        let _ = ShellExecuteW(
            None,
            PCWSTR(std::ptr::null()),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR(std::ptr::null()),
            PCWSTR(std::ptr::null()),
            SW_SHOW,
        );
    }
}

impl ImageViewerApp {
    fn load_folder(&mut self) {
        self.items.clear();
        self.texture_cache.clear();
        self.loading_set.clear();
        self.selected_item = None;
        self.loading = true;
        self.total_items = 0;
        
        let path = self.current_path.clone();
        let sender = self.image_sender.clone();
        
        std::thread::spawn(move || {
            let mut entries: Vec<FileSystemItem> = WalkDir::new(&path)
                .max_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.path() != Path::new(&path))
                .filter(|e| {
                    let entry_path = e.path();
                    
                    // Filter hidden/system files using Windows attributes
                    unsafe {
                        use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM, INVALID_FILE_ATTRIBUTES};
                        
                        let path_str = entry_path.to_string_lossy().to_string();
                        let path_wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();
                        let attrs = GetFileAttributesW(PCWSTR(path_wide.as_ptr()));
                        
                        // Skip if hidden or system file
                        if attrs != INVALID_FILE_ATTRIBUTES {
                            if (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0 || (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0 {
                                return false;
                            }
                        }
                    }
                    
                    // Also filter by name (belt and suspenders)
                    if let Some(name) = entry_path.file_name() {
                        let name_str = name.to_string_lossy();
                        // Skip hidden files (starts with .)
                        if name_str.starts_with('.') {
                            return false;
                        }
                        // Skip Windows system files
                        if matches!(name_str.to_lowercase().as_str(), 
                            "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information") {
                            return false;
                        }
                    }
                    
                    // For files, only show images and videos
                    if e.file_type().is_file() {
                        if let Some(ext) = e.path().extension() {
                            let ext_lower = ext.to_string_lossy().to_lowercase();
                            // Image formats
                            let is_image = matches!(ext_lower.as_str(), 
                                "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | 
                                "tiff" | "tif" | "ico" | "heic" | "heif" | "avif");
                            // Video formats
                            let is_video = matches!(ext_lower.as_str(),
                                "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | 
                                "webm" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts");
                            return is_image || is_video;
                        }
                        return false; // File without extension
                    }
                    
                    true // Keep directories
                })
                .map(|e| {
                    let path = e.path().to_path_buf();
                    if e.file_type().is_dir() {
                        FileSystemItem::Directory(path)
                    } else {
                        FileSystemItem::File(path)
                    }
                })
                .collect();
            
            // SORTING: Folders first, then alphabetically
            entries.sort_by(|a, b| {
                match (a, b) {
                    (FileSystemItem::Directory(_), FileSystemItem::File(_)) => Ordering::Less,
                    (FileSystemItem::File(_), FileSystemItem::Directory(_)) => Ordering::Greater,
                    _ => {
                        let a_name = a.path().file_name().unwrap_or_default();
                        let b_name = b.path().file_name().unwrap_or_default();
                        a_name.to_string_lossy().to_lowercase()
                            .cmp(&b_name.to_string_lossy().to_lowercase())
                    }
                }
            });
            
            for item in entries {
                let _ = sender.send(ThumbnailData {
                    path: item.path().to_path_buf(),
                    image_data: Vec::new(),
                    width: 0,
                    height: 0,
                });
            }
        });
    }
    
    fn navigate_to(&mut self, path: &str) {
        self.current_path = path.to_string();
        self.load_folder();
    }
    
    fn go_up_one_level(&mut self) {
        if let Some(parent) = Path::new(&self.current_path).parent() {
            self.current_path = parent.to_string_lossy().to_string();
            self.load_folder();
        }
    }
    
    fn request_thumbnail_load(&self, path: PathBuf) {
        let sender = self.image_sender.clone();
        
        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            
            let (thumbnail_data, width, height) = extract_windows_thumbnail(&path)
                .unwrap_or_else(|_| create_error_placeholder());
            
            let _ = sender.send(ThumbnailData {
                path,
                image_data: thumbnail_data,
                width,
                height,
            });
            
            unsafe {
                CoUninitialize();
            }
        });
    }
    
    fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        let mut received_any = false;
        
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            received_any = true;
            
            if thumbnail_data.image_data.is_empty() {
                let path = thumbnail_data.path.clone();
                if path.is_dir() {
                    self.items.push(FileSystemItem::Directory(path));
                } else {
                    self.items.push(FileSystemItem::File(path));
                }
                self.total_items = self.items.len();
            } else {
                self.loading_set.remove(&thumbnail_data.path);
                
                let texture = ctx.load_texture(
                    thumbnail_data.path.to_string_lossy().to_string(),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [thumbnail_data.width as usize, thumbnail_data.height as usize],
                        &thumbnail_data.image_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                
                self.texture_cache.put(thumbnail_data.path, texture);
            }
        }
        
        if received_any {
            ctx.request_repaint();
        }
    }
    
    fn render_item_slot(&mut self, ui: &mut egui::Ui, idx: usize) {
        if idx >= self.items.len() {
            return;
        }
        
        let item = &self.items[idx];
        let is_selected = self.selected_item == Some(idx);
        
        match item {
            FileSystemItem::Directory(path) => {
                let path_clone = path.clone();
                
                // Compact folder card with NO padding
                let frame = egui::Frame::none()
                    .fill(if is_selected {
                        egui::Color32::from_rgb(191, 228, 255)  // More visible Windows 11 blue
                    } else {
                        egui::Color32::from_gray(250)
                    })
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                    .rounding(4.0)
                    .inner_margin(0.0);  // NO padding!
                
                let response = frame.show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        // Folders are SMALLER - like Windows Explorer
                        let folder_icon_size = self.thumbnail_size * 0.6;
                        // Use full width for centering, but content is smaller
                        ui.set_width(self.thumbnail_size);
                        ui.set_height(folder_icon_size + 14.0);
                        
                        // Folder icon (smaller like Windows Explorer)
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("📁")
                                    .size(folder_icon_size * 0.7)
                                    .color(egui::Color32::from_rgb(255, 193, 7))
                            ).selectable(false)
                        );
                        
                        if let Some(name) = path_clone.file_name() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(name.to_string_lossy())
                                        .size(9.0)  // Smaller text for folders
                                        .color(egui::Color32::BLACK)
                                ).selectable(false)
                            );
                        }
                    });
                }).response;
                
                // FIXED: Use interact() with click sense for proper cursor
                let interact = response.interact(egui::Sense::click());
                
                if interact.clicked() {
                    self.selected_item = Some(idx);
                }
                
                if interact.double_clicked() {
                    self.navigate_to(&path_clone.to_string_lossy().to_string());
                }
            }
            FileSystemItem::File(path) => {
                let path_clone = path.clone();
                let has_texture = self.texture_cache.contains(&path_clone);
                let is_loading = self.loading_set.contains(&path_clone);
                
                if !has_texture && !is_loading && self.loading_set.len() < MAX_CONCURRENT_LOADS {
                    self.loading_set.insert(path_clone.clone());
                    self.request_thumbnail_load(path_clone.clone());
                }
                
                // Compact file card with NO padding
                let frame = egui::Frame::none()
                    .fill(if is_selected {
                        egui::Color32::from_rgb(191, 228, 255)  // More visible Windows 11 blue
                    } else {
                        egui::Color32::from_gray(250)
                    })
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(220)))
                    .rounding(4.0)
                    .inner_margin(0.0);  // NO padding!
                
                let response = frame.show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.set_width(self.thumbnail_size);
                        // Removed fixed height to let card wrap content tightly
                        // ui.set_height(self.thumbnail_size + 12.0);
                        
                        // Thumbnail with border
                        if let Some(texture) = self.texture_cache.get(&path_clone) {
                            ui.add(egui::Image::new(texture)
                                .max_size(egui::vec2(self.thumbnail_size, self.thumbnail_size))
                                .maintain_aspect_ratio(true)
                                .rounding(4.0));
                        } else {
                            egui::Frame::none()
                                .fill(egui::Color32::from_gray(240))
                                .rounding(4.0)
                                .show(ui, |ui| {
                                    ui.set_min_size(egui::vec2(self.thumbnail_size, self.thumbnail_size));
                                    ui.centered_and_justified(|ui| {
                                        ui.spinner();
                                    });
                                });
                        }
                        
                        if let Some(filename) = path_clone.file_name() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(filename.to_string_lossy())
                                        .size(10.0)
                                        .color(egui::Color32::BLACK)
                                ).selectable(false)  // Disable text selection
                            );
                        }
                    });
                }).response;
                
                // FIXED: Use interact() for proper double-click
                let interact = response.interact(egui::Sense::click());
                
                if interact.clicked() {
                    self.selected_item = Some(idx);
                }
                
                if interact.double_clicked() {
                    open_with_shell(&path_clone);
                }
            }
        }
    }
}

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_incoming_messages(ctx);
        
        // Windows 11 style sidebar
        egui::SidePanel::left("sidebar")
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);
                
                ui.heading("💾 Discos");
                ui.separator();
                
                for disk in &self.disks.clone() {
                    if ui.selectable_label(false, disk).clicked() {
                        self.navigate_to(disk);
                    }
                }
                
                ui.add_space(15.0);
                ui.heading("⭐ Atalhos");
                ui.separator();
                
                if let Ok(home) = env::var("USERPROFILE") {
                    if ui.selectable_label(false, "📷 Imagens").clicked() {
                        self.navigate_to(&format!("{}\\Pictures", home));
                    }
                    if ui.selectable_label(false, "🎬 Vídeos").clicked() {
                        self.navigate_to(&format!("{}\\Videos", home));
                    }
                    if ui.selectable_label(false, "📥 Downloads").clicked() {
                        self.navigate_to(&format!("{}\\Downloads", home));
                    }
                    if ui.selectable_label(false, "🗂️ Documentos").clicked() {
                        self.navigate_to(&format!("{}\\Documents", home));
                    }
                }
            });
        
        // Top navigation bar
        egui::TopBottomPanel::top("nav_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬅").clicked() {
                    self.go_up_one_level();
                }
                
                ui.separator();
                ui.label(format!("📂 {}", self.current_path));
            });
        });
        
        // Toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Zoom:");
                ui.add(egui::Slider::new(&mut self.thumbnail_size, 64.0..=256.0)
                    .show_value(false));
                
                ui.separator();
                ui.label(format!("Itens: {}", self.total_items));
                
                ui.separator();
                
                let memory_usage: usize = self.texture_cache.iter()
                    .map(|(_, tex)| {
                        let size = tex.size();
                        size[0] * size[1] * 4
                    })
                    .sum();
                
                ui.label(format!(
                    "VRAM: {:.1} MB",
                    memory_usage as f64 / 1024.0 / 1024.0
                ));
                
                if !self.loading_set.is_empty() {
                    ui.separator();
                    ui.spinner();
                }
            });
        });
        
        // Central grid
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.items.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    if self.loading {
                        ui.spinner();
                        ui.label("Carregando...");
                    } else {
                        ui.label("Pasta vazia");
                    }
                });
            } else {
                let available_width = ui.available_width();
                let item_width = self.thumbnail_size + 8.0;
                let cols = (available_width / item_width).max(1.0) as usize;
                let total_rows = (self.items.len() + cols - 1) / cols;
                let row_height = self.thumbnail_size + 20.0;
                
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_height, total_rows, |ui, row_range| {
                        // Use Grid for proper compact layout
                        egui::Grid::new("file_grid")
                            .spacing([4.0, 4.0])  // 4px spacing between items
                            .min_col_width(self.thumbnail_size + 4.0)
                            .max_col_width(self.thumbnail_size + 8.0)
                            .show(ui, |ui| {
                                for row in row_range.clone() {
                                    for col in 0..cols {
                                        let idx = row * cols + col;
                                        if idx < self.items.len() {
                                            self.render_item_slot(ui, idx);
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 720.0])
            .with_title("MTT File Manager"),
        ..Default::default()
    };
    
    eframe::run_native(
        "MTT File Manager",
        options,
        Box::new(|_cc| Ok(Box::new(ImageViewerApp::default()))),
    )
}
