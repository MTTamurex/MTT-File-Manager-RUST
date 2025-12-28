use eframe::egui::{self, UiStackInfo};
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

// Imports adicionaisexplícitos para APIs de ícones
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_LARGEICON, 
    SHGFI_USEFILEATTRIBUTES
};

// ...


/// Extrai ícone de "Este Computador" (This PC) usando PIDL (método robusto)
fn extract_computer_icon(size: IconSize) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // 1. Obtém o PIDL do "Meu Computador" (CSIDL_DRIVES)
        let pidl = match SHGetSpecialFolderLocation(HWND(std::ptr::null_mut()), CSIDL_DRIVES as i32) {
            Ok(p) => p,
            Err(_) => {
                println!("DEBUG: SHGetSpecialFolderLocation failed");
                return Err("Failed to get PIDL for My Computer".into());
            }
        };
        
        let mut shfi = SHFILEINFOW::default();
        
        // 2. Flags com SHGFI_PIDL (CRÍTICO!)
        let flags = SHGFI_PIDL | SHGFI_ICON | match size {
            IconSize::Small => SHGFI_SMALLICON,
            IconSize::Large => SHGFI_LARGEICON,
        };
        
        // 3. Pede o ícone usando o PIDL (cast para PCWSTR como exigido pela API)
        let result = SHGetFileInfoW(
            PCWSTR(pidl as *const u16),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        
        // 4. Limpa o PIDL (SEMPRE! Para evitar memory leak)
        CoTaskMemFree(Some(pidl as *const std::ffi::c_void));
        
        if result == 0 || shfi.hIcon.is_invalid() {
            println!("DEBUG: SHGetFileInfoW failed for PIDL");
            return Err("Failed to get computer icon".into());
        }
        
        // 5. Converte e limpa o ícone
        let hicon = shfi.hIcon;
        let conversion_result = hicon_to_rgba(hicon);
        
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
}

use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, DestroyIcon, ICONINFO, HICON};
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, GetVolumeInformationW};




// Caminho padrão
const PATH_PADRAO: &str = "C:\\";

// LRU cache - reduzido para limitar VRAM (~50-100MB)
const CACHE_SIZE: usize = 200;
const MAX_CONCURRENT_LOADS: usize = 30;  // Reduzido de 50

// Icon cache (menor pois ícones são compartilhados por extensão)
const ICON_CACHE_SIZE: usize = 100;

// Tamanho de ícones
#[derive(Copy, Clone)]
enum IconSize {
    Small,  // 16x16 ou 32x32 (depende do DPI)
    Large,  // 32x32 ou 48x48
}

// Modo de ordenação
#[derive(PartialEq, Clone, Copy, Debug)]
enum SortMode {
    Name,
    Date,
    Size,
}

// Entry de arquivo/pasta com metadados cacheados para ordenação
#[derive(Clone, Debug)]
struct FileEntry {
    path: PathBuf,
    name: String,      // Cache do nome para sort rápido
    is_dir: bool,      // Pastas primeiro
    size: u64,         // Tamanho em bytes (0 para diretórios)
    modified: u64,     // Timestamp (segundos desde UNIX_EPOCH)
}

impl FileEntry {
    fn from_path(path: PathBuf, is_dir: bool) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        
        // Tenta ler metadata, usa defaults em caso de erro (arquivos travados, etc)
        let (size, modified) = std::fs::metadata(&path)
            .ok()
            .map(|m| {
                let size = if is_dir { 0 } else { m.len() };
                let modified = m.modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                (size, modified)
            })
            .unwrap_or((0, 0));
        
        Self { path, name, is_dir, size, modified }
    }
    
    fn path(&self) -> &Path {
        &self.path
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
    items: Vec<FileEntry>,  // Agora com metadados cacheados
    texture_cache: LruCache<PathBuf, egui::TextureHandle>,
    loading_set: HashSet<PathBuf>,
    
    // Async loading (evita freeze da UI ao ler metadata)
    file_entry_receiver: Receiver<Vec<FileEntry>>,
    file_entry_sender: Sender<Vec<FileEntry>>,
    is_loading_folder: bool,
    
    // Icon cache (novo: extensão → texture)
    icon_cache: LruCache<String, egui::TextureHandle>,
    folder_icon_texture: Option<egui::TextureHandle>,
    computer_icon: Option<egui::TextureHandle>,  // Ícone "Este Computador"
    drive_icon_cache: LruCache<String, egui::TextureHandle>,  // path → icon
    
    // Sorting state
    sort_mode: SortMode,
    sort_descending: bool,  // true = Z-A, Mais Novo, Maior
    
    // Navigation state (histórico linear)
    navigation_history: Vec<String>,  // Histórico completo de paths
    history_index: usize,             // Posição atual no histórico
    path_input: String,               // Barra de endereço editável
    
    // UI state
    disks: Vec<(String, String)>,  // (path, label)
    thumbnail_size: f32,        // Zoom: 64-512
    selected_item: Option<usize>,
    
    total_items: usize,
}

impl Default for ImageViewerApp {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (file_entry_sender, file_entry_receiver) = mpsc::channel();
        let disks = get_all_drives();
        
        let mut app = Self {
            current_path: PATH_PADRAO.to_string(),
            image_sender: sender,
            image_receiver: receiver,
            items: Vec::new(),
            texture_cache: LruCache::new(NonZeroUsize::new(CACHE_SIZE).unwrap()),
            loading_set: HashSet::new(),
            // Async loading
            file_entry_receiver,
            file_entry_sender,
            is_loading_folder: false,
            icon_cache: LruCache::new(NonZeroUsize::new(ICON_CACHE_SIZE).unwrap()),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),  // Poucos drives
            // Sorting - padrão: Nome, Ascendente
            sort_mode: SortMode::Name,
            sort_descending: false,
            // Navigation - começa com path inicial no histórico
            navigation_history: vec![PATH_PADRAO.to_string()],
            history_index: 0,
            path_input: PATH_PADRAO.to_string(),
            disks,
            thumbnail_size: 128.0,  // Default zoom
            selected_item: None,
            total_items: 0,
        };
        
        app.load_folder();
        app
    }
}

/// Obtém o label (nome) de um volume do Windows.
/// Retorna "Disco Local" se não houver label ou falhar.
fn get_volume_label(drive_path: &str) -> String {
    unsafe {
        let path_wide: Vec<u16> = drive_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        let mut volume_name_buffer = vec![0u16; 256];
        
        let result = GetVolumeInformationW(
            PCWSTR(path_wide.as_ptr()),
            Some(&mut volume_name_buffer),
            None,
            None,
            None,
            None,
        );
        
        if result.is_ok() {
            let volume_name = String::from_utf16_lossy(&volume_name_buffer)
                .trim_end_matches('\0')
                .to_string();
            
            if !volume_name.is_empty() {
                return volume_name;
            }
        }
        
        "Disco Local".to_string()
    }
}

// Enumera drives com seus labels
fn get_all_drives() -> Vec<(String, String)> {
    unsafe {
        let mut buffer = vec![0u16; 256];
        let len = GetLogicalDriveStringsW(Some(&mut buffer));
        
        if len == 0 {
            return Vec::new();
        }
        
        String::from_utf16_lossy(&buffer[..len as usize])
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|path| {
                let label = get_volume_label(path);
                let drive_letter = path.trim_end_matches('\\');
                (path.to_string(), format!("{} ({})", label, drive_letter))
            })
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

/// Converte HICON para buffer RGBA.
/// Similar a hbitmap_to_rgba mas trabalha com ícones (que têm máscara).
/// 
/// # Safety
/// Usa GetIconInfo, GetDIBits. Não libera o HICON (responsabilidade do caller).
/// IMPORTANTE: Windows GDI retorna Pre-Multiplied Alpha. Tratamento adequado do canal alpha.
fn hicon_to_rgba(hicon: HICON) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // 1. Obtém estrutura ICONINFO (color bitmap + mask bitmap)
        let mut icon_info = ICONINFO::default();
        if GetIconInfo(hicon, &mut icon_info).is_err() {
            return Err("GetIconInfo failed".into());
        }
        
        let hbm_color = icon_info.hbmColor;
        
        // 2. Valida e converte o color bitmap
        let mut bm = BITMAP::default();
        GetObjectW(
            hbm_color,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        
        let width = bm.bmWidth as usize;
        let height = bm.bmHeight.abs() as usize;
        
        // 3. Valida tamanho (ícones costumam ser pequenos, mas defensivo)
        if width > 256 || height > 256 {
            // SAFETY: Cleanup antes de retornar erro
            let _ = DeleteObject(hbm_color);
            let _ = DeleteObject(icon_info.hbmMask);
            return Err("Icon too large".into());
        }
        
        let mut buffer = vec![0u8; width * height * 4];
        
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),  // Top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0 as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        
        let hdc = GetDC(None);
        let result = GetDIBits(
            hdc,
            hbm_color,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        
        // SAFETY: Sempre libera DC mesmo se GetDIBits falhar
        ReleaseDC(None, hdc);
        
        if result == 0 {
            // SAFETY: Cleanup antes de retornar erro
            let _ = DeleteObject(hbm_color);
            let _ = DeleteObject(icon_info.hbmMask);
            return Err("GetDIBits failed".into());
        }
        
        // 4. Cleanup dos bitmaps (mas NÃO do HICON - caller é responsável)
        let _ = DeleteObject(hbm_color);
        let _ = DeleteObject(icon_info.hbmMask);
        
        // 5. BGRA → RGBA conversion (Windows retorna BGRA)
        // NOTA: Alpha channel já está correto, apenas swap RGB channels
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);  // B ↔ R
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

/// Extrai ícone nativo do Windows para uma extensão de arquivo.
/// 
/// # Safety
/// Usa FFI para Windows APIs (SHGetFileInfoW, GetIconInfo, GetDIBits).
/// HICON deve ser sempre liberado com DestroyIcon.
/// 
/// CORREÇÃO: Usa FILE_ATTRIBUTE_NORMAL + SHGFI_USEFILEATTRIBUTES para obter ícone padrão do tipo.
fn extract_file_icon(
    extension: &str,  // ".pdf", ".exe", etc.
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // Cria path dummy com extensão (ex: "dummy.pdf")
        let dummy_path = format!("dummy{}", extension);
        let path_wide: Vec<u16> = dummy_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        let mut shfi = SHFILEINFOW::default();
        
        // FLAGS CORRETAS: USEFILEATTRIBUTES permite usar path dummy
        let flags = SHGFI_ICON 
            | SHGFI_USEFILEATTRIBUTES  // Não precisa do arquivo existir
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large => SHGFI_LARGEICON,
            };
        
        // SAFETY: SHGetFileInfoW retorna handle que DEVE ser destruído
        // O Pulo do Gato: FILE_ATTRIBUTE_NORMAL para arquivos genéricos
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,  // Atributo para arquivo normal
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        
        if result == 0 || shfi.hIcon.is_invalid() {
            println!("DEBUG: SHGetFileInfo falhou para: {}", dummy_path);
            return Err("Failed to get file icon".into());
        }
        
        let hicon = shfi.hIcon;
        
        // Converte HICON → RGBA
        let conversion_result = hicon_to_rgba(hicon);
        
        // SAFETY: Sempre libera HICON (RAII pattern)
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
}

/// Extrai ícone de pasta usando path DUMMY (não real).
/// 
/// CORREÇÃO: Usa FILE_ATTRIBUTE_DIRECTORY + SHGFI_USEFILEATTRIBUTES + path dummy
/// para obter o ícone padrão de pasta do Windows.
fn extract_folder_icon_internal(
    _folder_path: &str,  // Ignorado - usamos dummy
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // O Pulo do Gato: usar path DUMMY, não real!
        let dummy_path = "dummy_folder";
        let path_wide: Vec<u16> = dummy_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        let mut shfi = SHFILEINFOW::default();
        
        // FLAGS CORRETAS: USEFILEATTRIBUTES permite usar path dummy
        let flags = SHGFI_ICON 
            | SHGFI_USEFILEATTRIBUTES  // Permite path dummy
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large => SHGFI_LARGEICON,
            };
        
        // SAFETY: SHGetFileInfoW com path dummy
        // O Pulo do Gato: FILE_ATTRIBUTE_DIRECTORY no parâmetro dwFileAttributes!
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_DIRECTORY,  // Indica que é uma pasta
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        
        if result == 0 || shfi.hIcon.is_invalid() {
            println!("DEBUG: SHGetFileInfo falhou para pasta dummy");
            return Err("Failed to get folder icon".into());
        }
        
        let hicon = shfi.hIcon;
        let conversion_result = hicon_to_rgba(hicon);
        
        // SAFETY: Sempre libera HICON
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
}

/// Extrai ícone REAL de um drive (C:\, D:\, etc.).
/// 
/// DIFERENÇA: Usa path REAL (não dummy) e SEM SHGFI_USEFILEATTRIBUTES.
/// Isso força o Windows a retornar o ícone específico do drive (HD, SSD, USB, etc.).
fn extract_drive_icon(
    drive_path: &str,  // Deve ter barra: "C:\\"
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_wide: Vec<u16> = drive_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        let mut shfi = SHFILEINFOW::default();
        
        // FLAGS: Sem USEFILEATTRIBUTES - queremos ícone REAL do volume
        let flags = SHGFI_ICON 
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large => SHGFI_LARGEICON,
            };
        
        // SAFETY: SHGetFileInfoW com path real de drive
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,  // Use NORMAL para deixar Windows detectar tipo
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        
        if result == 0 || shfi.hIcon.is_invalid() {
            println!("DEBUG: SHGetFileInfo falhou para drive: {}", drive_path);
            return Err("Failed to get drive icon".into());
        }
        
        let hicon = shfi.hIcon;
        let conversion_result = hicon_to_rgba(hicon);
        
        // SAFETY: Sempre libera HICON
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
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
    /// Ordena itens baseado no modo atual (mantém pastas sempre primeiro)
    fn sort_items(&mut self) {
        self.items.sort_by(|a, b| {
            // 1. Pastas sempre primeiro (a menos que ambos sejam pastas ou ambos arquivos)
            if a.is_dir != b.is_dir {
                return if a.is_dir {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            
            // 2. Ordena por modo selecionado
            let ordering = match self.sort_mode {
                SortMode::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortMode::Date => a.modified.cmp(&b.modified),
                SortMode::Size => a.size.cmp(&b.size),
            };
            
            // 3. Inverte se descending está ativo
            if self.sort_descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    
    fn load_folder(&mut self) {
        self.items.clear();
        self.texture_cache.clear();
        self.loading_set.clear();
        self.selected_item = None;
        self.is_loading_folder = true;  // Mudou de loading
        self.total_items = 0;
        
        let path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        
        // Background thread: lê metadata SEM bloquear UI
        std::thread::spawn(move || {
            let entries: Vec<FileEntry> = WalkDir::new(&path)
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
                    
                    // NEW: Mostra TODOS os arquivos (ícones tratam visualização)
                    // Pastas e arquivos de qualquer tipo passam pelo filtro
                    true
                })
                .map(|e| {
                    let path = e.path().to_path_buf();
                    let is_dir = e.file_type().is_dir();
                    // Cria FileEntry com metadados cacheados
                    FileEntry::from_path(path, is_dir)
                })
                .collect();
            
            // Envia batch completo (já com metadata) para UI thread
            let _ = file_entry_sender.send(entries);
        });
    }
    
    /// Navega para um caminho, adicionando ao histórico (corta histórico futuro)
    fn navigate_to(&mut self, path: &str) {
        // Se já estamos nesse caminho, não faz nada
        if self.current_path == path {
            return;
        }
        
        // Corta histórico "futuro" (se voltamos e navegamos para outro lugar)
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }
        
        // Adiciona novo caminho ao histórico
        self.navigation_history.push(path.to_string());
        self.history_index = self.navigation_history.len() - 1;
        
        self.current_path = path.to_string();
        self.path_input = path.to_string();
        self.load_folder();
    }
    
    /// Volta no histórico (sem adicionar ao histórico)
    fn go_back(&mut self) {
        if self.can_go_back() {
            self.history_index -= 1;
            self.current_path = self.navigation_history[self.history_index].clone();
            self.path_input = self.current_path.clone();
            self.load_folder();
        }
    }
    
    /// Avança no histórico
    fn go_forward(&mut self) {
        if self.can_go_forward() {
            self.history_index += 1;
            self.current_path = self.navigation_history[self.history_index].clone();
            self.path_input = self.current_path.clone();
            self.load_folder();
        }
    }
    
    /// Sobe um nível (adiciona ao histórico)
    fn go_up_one_level(&mut self) {
        if let Some(parent) = Path::new(&self.current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() {
                self.navigate_to(&parent_str);
            }
        }
    }
    
    /// Pode voltar no histórico?
    fn can_go_back(&self) -> bool {
        self.history_index > 0
    }
    
    /// Pode avançar no histórico?
    fn can_go_forward(&self) -> bool {
        self.history_index < self.navigation_history.len().saturating_sub(1)
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
    
    /// Retorna ícone para uma extensão, carregando sob demanda.
    fn get_or_load_icon(
        &mut self, 
        ctx: &egui::Context,
        extension: &str,
    ) -> Option<egui::TextureHandle> {
        let key = extension.to_lowercase();
        
        // Cache hit? Clone do handle (barato)
        if let Some(texture) = self.icon_cache.get(&key) {
            return Some(texture.clone());
        }
        
        // Cache miss → carrega ícone
        // Captura thumbnail_size antes de borrowar mutavelmente self
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };
        
        match extract_file_icon(&key, icon_size) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("icon_{}", key),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                
                let cloned = texture.clone();
                self.icon_cache.put(key, texture);
                Some(cloned)
            }
            Err(_) => None,  // Fallback: sem ícone
        }
    }
    
    /// Garante que ícone de pasta está carregado.
    fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        if self.folder_icon_texture.is_some() {
            return; // Já carregado
        }
        
        // Windows usa ícone especial para pastas
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };
        
        // Truque: usa "C:\\" que sempre existe
        match extract_folder_icon_internal("C:\\", icon_size) {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    "folder_icon",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                self.folder_icon_texture = Some(texture);
            }
            Err(_) => {
                // Fallback: mantém emoji
            }
        }
    }
    
    /// Garante que ícone de "Este Computador" está carregado.
    fn ensure_computer_icon(&mut self, ctx: &egui::Context) {
        if self.computer_icon.is_some() {
            return;
        }
        
        if let Ok((data, width, height)) = extract_computer_icon(IconSize::Small) {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                &data,
            );
            
            self.computer_icon = Some(ctx.load_texture(
                "computer_icon",
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
    }
    
    fn process_incoming_messages(&mut self, ctx: &egui::Context) {
        // 1. Batch FileEntry loading (evita freeze)
        if let Ok(entries) = self.file_entry_receiver.try_recv() {
            self.items = entries;
            self.sort_items();
            self.total_items = self.items.len();
            self.is_loading_folder = false;
            ctx.request_repaint();
        }
        
        // 2. Individual thumbnails
        let mut received_any = false;
        let mut new_items_added = false;
        
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            received_any = true;
            
            // Só processa thumbnails (image_data não vazio)
            if !thumbnail_data.image_data.is_empty() {
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
        
        let item = self.items[idx].clone();  // Clone para evitar borrow conflicts
        let is_selected = self.selected_item == Some(idx);
        
        // ==== DIRECTORY RENDERING ====
        if item.is_dir {
            let path_clone = item.path.clone();
                
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
                        
                        // NEW: Tenta usar ícone nativo, fallback para emoji
                        if let Some(icon_texture) = &self.folder_icon_texture {
                            ui.add(egui::Image::new(icon_texture)
                                .max_size(egui::vec2(folder_icon_size, folder_icon_size))
                                .maintain_aspect_ratio(true));
                        } else {
                            // Fallback: emoji atual
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new("📁")
                                        .size(folder_icon_size * 0.7)
                                        .color(egui::Color32::from_rgb(255, 193, 7))
                                ).selectable(false)
                            );
                        }
                        
                        // PERFORMANCE: Usa item.name (já cacheado) em vez de path.file_name()
                        ui.set_min_height(20.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&item.name)
                                    .size(9.0)  // Smaller text for folders
                                    .color(egui::Color32::BLACK)
                            )
                            .wrap()
                            .truncate()
                            .selectable(false)
                        );
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
            // ==== FILE RENDERING ====
            else {
                let path_clone = item.path.clone();
                
                // Detecta se é arquivo de mídia
                let is_media_file = if let Some(ext) = path_clone.extension() {
                    let ext_lower = ext.to_string_lossy().to_lowercase();
                    matches!(ext_lower.as_str(),
                        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
                        "tiff" | "tif" | "ico" | "heic" | "heif" | "avif" |
                        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" |
                        "webm" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts"
                    )
                } else {
                    false
                };
                
                // Thumbnail loading apenas para arquivos de mídia
                if is_media_file {
                    let has_texture = self.texture_cache.contains(&path_clone);
                    let is_loading = self.loading_set.contains(&path_clone);
                    
                    if !has_texture && !is_loading && self.loading_set.len() < MAX_CONCURRENT_LOADS {
                        self.loading_set.insert(path_clone.clone());
                        self.request_thumbnail_load(path_clone.clone());
                    }
                }
                
                // PRÉ-CARREGA ícone para arquivos não-mídia ANTES de entrar no closure
                let file_icon = if !is_media_file {
                    if let Some(ext) = path_clone.extension() {
                        let ext_str = format!(".{}", ext.to_string_lossy());
                        self.get_or_load_icon(ui.ctx(), &ext_str)
                    } else {
                        None
                    }
                } else {
                    None
                };

                
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
                        
                        if is_media_file {
                            // EXISTING: Lógica de thumbnail para arquivos de mídia
                            if let Some(texture) = self.texture_cache.get(&path_clone) {
                                ui.add(egui::Image::new(texture)
                                    .max_size(egui::vec2(self.thumbnail_size, self.thumbnail_size))
                                    .maintain_aspect_ratio(true)
                                    .rounding(4.0));
                            } else {
                                // Loading spinner
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
                        } else {
                            // NEW: Arquivo não-mídia → ícone do sistema (pré-carregado)
                            if let Some(icon_texture) = file_icon {
                                // Ícones são pequenos, centraliza
                                let icon_display_size = self.thumbnail_size * 0.5;
                                ui.add_space((self.thumbnail_size - icon_display_size) / 2.0);
                                ui.add(egui::Image::new(&icon_texture)
                                    .max_size(egui::vec2(icon_display_size, icon_display_size))
                                    .maintain_aspect_ratio(true));
                            } else {
                                // Fallback: emoji genérico
                                ui.set_min_height(self.thumbnail_size);
                                ui.centered_and_justified(|ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new("📄")
                                                .size(self.thumbnail_size * 0.4)
                                                .color(egui::Color32::GRAY)
                                        ).selectable(false)
                                    );
                                });
                            }
                        }
                        
                        // PERFORMANCE: Usa item.name (já cacheado) em vez de path.file_name()
                        ui.set_min_height(20.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&item.name)
                                    .size(10.0)
                                    .color(egui::Color32::BLACK)
                            )
                            .wrap()
                            .truncate()
                            .selectable(false)
                        );
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

impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_incoming_messages(ctx);
        self.ensure_folder_icon(ctx);
        self.ensure_computer_icon(ctx);  // Carrega ícone "Este Computador"

        
        // Windows 11 style sidebar
        egui::SidePanel::left("sidebar")
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);
                
                // Header "Este Computador" com ícone nativo
                ui.horizontal(|ui| {
                    if let Some(icon) = &self.computer_icon {
                        ui.add(egui::Image::new(icon)
                            .max_size(egui::vec2(16.0, 16.0))
                            .maintain_aspect_ratio(true));
                    }
                    ui.label(egui::RichText::new("Este Computador").strong().size(16.0));
                });
                
                ui.separator();
                
                ui.add_space(5.0);  // Espaçamento superior
                
                
                for (disk_path, disk_label) in &self.disks.clone() {
                    // Pré-carrega ícone do drive se não estiver no cache
                    let drive_icon = if let Some(icon) = self.drive_icon_cache.get(disk_path) {
                        Some(icon.clone())
                    } else {
                        // Tenta carregar ícone real do drive
                        if let Ok((rgba_data, width, height)) = extract_drive_icon(disk_path, IconSize::Small) {
                            let texture = ui.ctx().load_texture(
                                format!("drive_{}", disk_path),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [width as usize, height as usize],
                                    &rgba_data,
                                ),
                                egui::TextureOptions::LINEAR,
                            );
                            let cloned = texture.clone();
                            self.drive_icon_cache.put(disk_path.clone(), texture);
                            Some(cloned)
                        } else {
                            None
                        }
                    };
                    
                    // Renderiza drive com ícone + label
                    let response = ui.horizontal(|ui| {
                        if let Some(icon) = drive_icon {
                            ui.add(egui::Image::new(&icon).max_size(egui::vec2(16.0, 16.0)));
                        } else {
                            ui.label("💾");  // Fallback
                        }
                        ui.selectable_label(false, disk_label)
                    }).inner;
                    
                    if response.clicked() {
                        self.navigate_to(disk_path);
                    }
                    
                    ui.add_space(3.0);  // Espaçamento entre drives
                }
            });
        
        // Top navigation bar
        egui::TopBottomPanel::top("nav_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Botão Voltar (desabilitado se não pode voltar)
                let can_back = self.can_go_back();
                if ui.add_enabled(can_back, egui::Button::new("⬅")).clicked() {
                    self.go_back();
                }
                
                // Botão Avançar (desabilitado se não pode avançar)
                let can_forward = self.can_go_forward();
                if ui.add_enabled(can_forward, egui::Button::new("➡")).clicked() {
                    self.go_forward();
                }
                
                // Botão Subir
                if ui.button("⬆").clicked() {
                    self.go_up_one_level();
                }
                
                ui.separator();
                
                // Sorting controls (BEFORE address bar to ensure visibility)
                ui.label("Ordenar:");
                egui::ComboBox::from_id_source("sort_mode")
                    .selected_text(match self.sort_mode {
                        SortMode::Name => "Nome",
                        SortMode::Date => "Data",
                        SortMode::Size => "Tamanho",
                    })
                    .show_ui(ui, |ui| {
                        if ui.selectable_value(&mut self.sort_mode, SortMode::Name, "Nome").clicked() { 
                            self.sort_items(); 
                        }
                        if ui.selectable_value(&mut self.sort_mode, SortMode::Date, "Data").clicked() { 
                            self.sort_items(); 
                        }
                        if ui.selectable_value(&mut self.sort_mode, SortMode::Size, "Tamanho").clicked() { 
                            self.sort_items(); 
                        }
                    });
                
                // Toggle ascending/descending
                let sort_icon = if self.sort_descending { "⬇" } else { "⬆" };
                if ui.button(sort_icon).clicked() {
                    self.sort_descending = !self.sort_descending;
                    self.sort_items();
                }
                
                ui.separator();
                
                // Barra de endereço editável
                let response = ui.add_sized(
                    egui::vec2(ui.available_width() - 10.0, 20.0),
                    egui::TextEdit::singleline(&mut self.path_input)
                        .font(egui::TextStyle::Monospace)
                );
                
                // Enter para navegar
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let path = self.path_input.clone();
                    if Path::new(&path).exists() {
                        self.navigate_to(&path);
                    } else {
                        // Restaura o path atual se inválido
                        self.path_input = self.current_path.clone();
                    }
                }
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
            // 1. PRIORIDADE: Carregando? Mostra spinner
            if self.is_loading_folder {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.spinner();
                    ui.label("Carregando...");
                });
            }
            // 2. Lista vazia (e não carregando)? Mostra mensagem
            else if self.items.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(100.0);
                    ui.label("Pasta vazia");
                });
            }
            // 3. Tem items? Renderiza grid
            else {
                // ============================================
                // POSICIONAMENTO ABSOLUTO (Game Engine Style)
                // Elimina 100% do jitter usando coordenadas matemáticas
                // ============================================
                
                // 1. GEOMETRIA FIXA (constantes rígidas)
                let padding = 8.0;
                let item_w = self.thumbnail_size;
                let item_h = self.thumbnail_size + 40.0;  // Altura RÍGIDA (Thumb + Texto)
                let available_w = ui.available_width();
                let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
                
                // 2. CÁLCULO DA ALTURA TOTAL (Virtual Scroll)
                let count = self.items.len();
                let rows = (count as f32 / cols as f32).ceil() as usize;
                let total_height = rows as f32 * (item_h + padding) + padding;
                
                // 3. SCROLL AREA (sem show_rows - controle manual)
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // A) Força altura total para barra de rolagem correta
                        let content_min = ui.min_rect().min;
                        ui.allocate_rect(
                            egui::Rect::from_min_size(content_min, egui::vec2(available_w, total_height)),
                            egui::Sense::hover()
                        );
                        
                        // B) Descobre viewport visível para virtualização
                        let clip_rect = ui.clip_rect();
                        let start_y = (clip_rect.top() - content_min.y).max(0.0);
                        let end_y = start_y + clip_rect.height();
                        
                        // C) Calcula quais linhas desenhar (otimização)
                        let min_row = (start_y / (item_h + padding)).floor() as usize;
                        let max_row = ((end_y / (item_h + padding)).ceil() as usize + 1).min(rows);
                        
                        // D) Loop de desenho absoluto - posição matemática exata
                        for row in min_row..max_row {
                            for col in 0..cols {
                                let index = row * cols + col;
                                if index >= count { break; }
                                
                                // Cálculo matemático da posição (X, Y)
                                let x_pos = col as f32 * (item_w + padding) + padding;
                                let y_pos = row as f32 * (item_h + padding) + padding;
                                
                                // Cria retângulo onde o card VAI morar
                                let rect = egui::Rect::from_min_size(
                                    content_min + egui::vec2(x_pos, y_pos),
                                    egui::vec2(item_w, item_h)
                                );
                                
                                // CULLING ESTRITO: Pula items fora do viewport
                                if !ui.is_rect_visible(rect) {
                                    continue; // Não carrega thumbnail nem renderiza
                                }
                                
                                // Desenha o card naquela posição EXATA
                                ui.allocate_ui_at_rect(rect, |ui| {
                                    self.render_item_slot(ui, index);
                                }).response;
                            }
                        }
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
