use eframe::egui;
use lru::LruCache;
use std::cmp::Ordering;
use std::collections::HashSet;
// use std::env;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::Storage::FileSystem::*,
    Win32::System::Com::*,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
};

// Imports adicionais explÃ­citos para APIs de Ã­cones
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_LARGEICON, 
    SHGFI_USEFILEATTRIBUTES, SHGFI_DISPLAYNAME
};

// OTIMIZAÃ‡ÃƒO: Imports para Win32 FindFirst/NextFileW (metadata em UMA syscall)
use windows::Win32::Storage::FileSystem::{
    FindFirstFileW, FindNextFileW, FindClose, WIN32_FIND_DATAW, FILE_ATTRIBUTE_DIRECTORY
};
use std::os::windows::ffi::OsStringExt;

// ...


/// Extrai Ã­cone de "Este Computador" (This PC) usando PIDL (mÃ©todo robusto)
fn extract_computer_icon(size: IconSize) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // 1. ObtÃ©m o PIDL do "Meu Computador" (CSIDL_DRIVES)
        let pidl = match SHGetSpecialFolderLocation(HWND(std::ptr::null_mut()), CSIDL_DRIVES as i32) {
            Ok(p) => p,
            Err(_) => {
                println!("DEBUG: SHGetSpecialFolderLocation failed");
                return Err("Failed to get PIDL for My Computer".into());
            }
        };
        
        let mut shfi = SHFILEINFOW::default();
        
        // 2. Flags com SHGFI_PIDL (CRÃTICO!)
        let flags = SHGFI_PIDL | SHGFI_ICON | match size {
            IconSize::Small => SHGFI_SMALLICON,
            IconSize::Large => SHGFI_LARGEICON,
        };
        
        // 3. Pede o Ã­cone usando o PIDL (cast para PCWSTR como exigido pela API)
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
        
        // 5. Converte e limpa o Ã­cone
        let hicon = shfi.hIcon;
        let conversion_result = hicon_to_rgba(hicon);
        
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
}

use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, DestroyIcon, ICONINFO, HICON};
// FILE_ATTRIBUTE_DIRECTORY jÃ¡ importado acima, GetVolumeInformationW mantido
use windows::Win32::Storage::FileSystem::GetVolumeInformationW;




// Caminho padrÃ£o
const PATH_PADRAO: &str = "C:\\";

// LRU cache - reduzido para limitar VRAM (~50-100MB)
const CACHE_SIZE: usize = 200;
const MAX_CONCURRENT_LOADS: usize = 30;  // Reduzido de 50
const PRELOAD_ROWS: usize = 5;  // Pre-fetch: carrega 5 linhas antes/depois da viewport


// Icon cache (menor pois Ã­cones sÃ£o compartilhados por extensÃ£o)
const ICON_CACHE_SIZE: usize = 100;

// Tamanho de Ã­cones
#[derive(Copy, Clone)]
enum IconSize {
    Small,  // 16x16 ou 32x32 (depende do DPI)
    Large,  // 32x32 ou 48x48
}

// Modo de ordenaÃ§Ã£o
#[derive(PartialEq, Clone, Copy, Debug)]
enum SortMode {
    Name,
    Date,
    Size,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum ViewMode {
    Grid,
    List,
}

/// Busca primeira imagem em uma pasta para usar como preview
/// Verifica apenas os primeiros 15 arquivos para performance
fn find_first_image_in_folder(folder_path: &Path) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(folder_path) {
        for entry in entries.flatten().take(15) {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    match ext.to_lowercase().as_str() {
                        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" => return Some(path),
                        _ => {}
                    }
                }
            }
        }
    }
    None
}

// Entry de arquivo/pasta com metadados cacheados para ordenaÃ§Ã£o
#[derive(Clone, Debug)]
struct FileEntry {
    path: PathBuf,
    name: String,      // Cache do nome para sort rÃ¡pido
    is_dir: bool,      // Pastas primeiro
    size: u64,         // Tamanho em bytes (0 para diretÃ³rios)
    modified: u64,     // Timestamp (segundos desde UNIX_EPOCH)
    folder_cover: Option<PathBuf>,  // Primeira imagem encontrada na pasta (para preview)
}

/// Helper para exibir tipo do arquivo na Lista
fn get_file_type_string(entry: &FileEntry) -> String {
    if entry.is_dir {
        return "Pasta".to_string();
    }
    if let Some(ext) = entry.path.extension() {
        return format!("Arquivo {}", ext.to_string_lossy().to_uppercase());
    }
    "Arquivo".to_string()
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
        
        // OTIMIZAÃ‡ÃƒO: Lazy loading - sempre None inicialmente.
        // O scan serÃ¡ disparado por request_folder_scan() quando a pasta ficar visÃ­vel.
        let folder_cover = None;
        
        Self { path, name, is_dir, size, modified, folder_cover }
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

// AplicaÃ§Ã£o principal
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
    
    // COVER WORKER: Sistema de capas de pasta (Single Thread Worker)
    cover_worker_sender: Sender<PathBuf>,  // UI â†’ Worker: Envia pasta para processar
    cover_worker_receiver: Receiver<(PathBuf, Option<PathBuf>)>,  // Worker â†’ UI: Resultado
    scanned_folders: HashSet<PathBuf>,  // Cache: evita re-scan
    
    // Icon cache (novo: extensÃ£o â†’ texture)
    icon_cache: LruCache<String, egui::TextureHandle>,
    folder_icon_texture: Option<egui::TextureHandle>,
    computer_icon: Option<egui::TextureHandle>,  // Ãcone "Este Computador"
    drive_icon_cache: LruCache<String, egui::TextureHandle>,  // path â†’ icon
    
    // Sorting state
    sort_mode: SortMode,
    sort_descending: bool,  // true = Z-A, Mais Novo, Maior
    
    // View Mode
    view_mode: ViewMode,
    
    // Navigation state (histÃ³rico linear)
    navigation_history: Vec<String>,  // HistÃ³rico completo de paths
    history_index: usize,             // PosiÃ§Ã£o atual no histÃ³rico
    path_input: String,               // Barra de endereÃ§o editÃ¡vel
    
    // UI state
    disks: Vec<(String, String)>,  // (path, label)
    thumbnail_size: f32,        // Zoom: 64-512
    selected_item: Option<usize>,
    selected_file: Option<FileEntry>,
    show_preview_panel: bool,
    
    total_items: usize,
    
    // Search & Navigation (NEW)
    all_items: Vec<FileEntry>,  // Cache mestre para busca
    search_query: String,       // Texto da busca
    last_grid_cols: usize,      // MemÃ³ria para navegaÃ§Ã£o vertical (teclado)
}

impl Default for ImageViewerApp {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        let (file_entry_sender, file_entry_receiver) = mpsc::channel();
        
        // COVER WORKER: Worker Ãºnico para processar capas de pasta
        let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>();  // UI â†’ Worker
        let (cover_res_tx, cover_res_rx) = mpsc::channel();             // Worker â†’ UI
        
        // Spawna WORKER THREAD: fica em loop processando fila
        std::thread::spawn(move || {
            // Loop infinito: consome requisiÃ§Ãµes da fila
            while let Ok(folder_path) = cover_req_rx.recv() {
                // Executa busca (funÃ§Ã£o jÃ¡ existente)
                let cover = find_first_image_in_folder(&folder_path);
                
                // Devolve resultado para UI thread
                let _ = cover_res_tx.send((folder_path, cover));
            }
        });
        
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
            // Cover Worker
            cover_worker_sender: cover_req_tx,
            cover_worker_receiver: cover_res_rx,
            scanned_folders: HashSet::new(),
            icon_cache: LruCache::new(NonZeroUsize::new(ICON_CACHE_SIZE).unwrap()),
            folder_icon_texture: None,
            computer_icon: None,
            drive_icon_cache: LruCache::new(NonZeroUsize::new(10).unwrap()),  // Poucos drives
            // Sorting - padrÃ£o: Nome, Ascendente
            sort_mode: SortMode::Name,
            sort_descending: false,
            // View mode: Grid por padrÃ£o
            view_mode: ViewMode::Grid,
            // Selection & Preview
            selected_file: None,
            show_preview_panel: true,  // Mostrar por padrÃ£o
            // Navigation - comeÃ§a com path inicial no histÃ³rico
            navigation_history: vec![PATH_PADRAO.to_string()],
            history_index: 0,
            path_input: PATH_PADRAO.to_string(),
            disks,
            thumbnail_size: 128.0,  // Default zoom
            selected_item: None,
            total_items: 0,
            // Search & Navigation (NEW)
            all_items: Vec::new(),
            search_query: String::new(),
            last_grid_cols: 1,
        };
        
        app.load_folder();
        app
    }
}

/// ObtÃ©m o label (nome) de um volume do Windows.
/// Usa Shell Display Name (suporta drives virtuais como Cryptomator).
/// Fallback para GetVolumeInformationW se Shell falhar.
fn get_volume_label(drive_path: &str) -> String {
    unsafe {
        let path_wide: Vec<u16> = drive_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        // Primeiro: tenta Shell Display Name (suporta Cryptomator, etc)
        let mut shfi: SHFILEINFOW = std::mem::zeroed();
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_DIRECTORY,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        );
        
        if result != 0 {
            let display_name = String::from_utf16_lossy(&shfi.szDisplayName)
                .trim_end_matches('\0')
                .to_string();
            
            // Shell retorna "Label (X:)" - extraimos sÃ³ o label
            if let Some(paren_pos) = display_name.rfind(" (") {
                let label = display_name[..paren_pos].trim();
                if !label.is_empty() {
                    return label.to_string();
                }
            } else if !display_name.is_empty() {
                return display_name;
            }
        }
        
        // Fallback: GetVolumeInformationW (volume label real)
        let mut volume_name_buffer = vec![0u16; 256];
        let vol_result = GetVolumeInformationW(
            PCWSTR(path_wide.as_ptr()),
            Some(&mut volume_name_buffer),
            None,
            None,
            None,
            None,
        );
        
        if vol_result.is_ok() {
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
/// Similar a hbitmap_to_rgba mas trabalha com Ã­cones (que tÃªm mÃ¡scara).
/// 
/// # Safety
/// Usa GetIconInfo, GetDIBits. NÃ£o libera o HICON (responsabilidade do caller).
/// IMPORTANTE: Windows GDI retorna Pre-Multiplied Alpha. Tratamento adequado do canal alpha.
fn hicon_to_rgba(hicon: HICON) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // 1. ObtÃ©m estrutura ICONINFO (color bitmap + mask bitmap)
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
        
        // 3. Valida tamanho (Ã­cones costumam ser pequenos, mas defensivo)
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
        
        // 4. Cleanup dos bitmaps (mas NÃƒO do HICON - caller Ã© responsÃ¡vel)
        let _ = DeleteObject(hbm_color);
        let _ = DeleteObject(icon_info.hbmMask);
        
        // 5. BGRA â†’ RGBA conversion (Windows retorna BGRA)
        // NOTA: Alpha channel jÃ¡ estÃ¡ correto, apenas swap RGB channels
        for pixel in buffer.chunks_exact_mut(4) {
            pixel.swap(0, 2);  // B â†” R
        }
        
        Ok((buffer, width as u32, height as u32))
    }
}

/// Extrai Ã­cone nativo do Windows para uma extensÃ£o de arquivo.
/// 
/// # Safety
/// Usa FFI para Windows APIs (SHGetFileInfoW, GetIconInfo, GetDIBits).
/// HICON deve ser sempre liberado com DestroyIcon.
/// 
/// CORREÃ‡ÃƒO: Usa FILE_ATTRIBUTE_NORMAL + SHGFI_USEFILEATTRIBUTES para obter Ã­cone padrÃ£o do tipo.
fn extract_file_icon(
    extension: &str,  // ".pdf", ".exe", etc.
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // Cria path dummy com extensÃ£o (ex: "dummy.pdf")
        let dummy_path = format!("dummy{}", extension);
        let path_wide: Vec<u16> = dummy_path
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        let mut shfi = SHFILEINFOW::default();
        
        // FLAGS CORRETAS: USEFILEATTRIBUTES permite usar path dummy
        let flags = SHGFI_ICON 
            | SHGFI_USEFILEATTRIBUTES  // NÃ£o precisa do arquivo existir
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large => SHGFI_LARGEICON,
            };
        
        // SAFETY: SHGetFileInfoW retorna handle que DEVE ser destruÃ­do
        // O Pulo do Gato: FILE_ATTRIBUTE_NORMAL para arquivos genÃ©ricos
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
        
        // Converte HICON â†’ RGBA
        let conversion_result = hicon_to_rgba(hicon);
        
        // SAFETY: Sempre libera HICON (RAII pattern)
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
}

/// Extrai Ã­cone de pasta usando path DUMMY (nÃ£o real).
/// 
/// CORREÃ‡ÃƒO: Usa FILE_ATTRIBUTE_DIRECTORY + SHGFI_USEFILEATTRIBUTES + path dummy
/// para obter o Ã­cone padrÃ£o de pasta do Windows.
fn extract_folder_icon_internal(
    _folder_path: &str,  // Ignorado - usamos dummy
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        // O Pulo do Gato: usar path DUMMY, nÃ£o real!
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
        // O Pulo do Gato: FILE_ATTRIBUTE_DIRECTORY no parÃ¢metro dwFileAttributes!
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_DIRECTORY,  // Indica que Ã© uma pasta
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

/// Extrai icone de um arquivo REAL usando path completo.
/// Usado para executaveis (.exe, .lnk, .ico) que tem icones unicos.
fn extract_file_icon_by_path(
    path: &Path,
    size: IconSize,
) -> std::result::Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    unsafe {
        let path_wide: Vec<u16> = path.to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        
        let mut shfi = SHFILEINFOW::default();
        
        // SEM USEFILEATTRIBUTES - usa arquivo real
        let flags = SHGFI_ICON 
            | match size {
                IconSize::Small => SHGFI_SMALLICON,
                IconSize::Large => SHGFI_LARGEICON,
            };
        
        // SAFETY: SHGetFileInfoW com path real
        let result = SHGetFileInfoW(
            PCWSTR(path_wide.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        
        if result == 0 || shfi.hIcon.is_invalid() {
            return Err("Failed to get file icon".into());
        }
        
        let hicon = shfi.hIcon;
        let conversion_result = hicon_to_rgba(hicon);
        
        // SAFETY: Sempre libera HICON
        let _ = DestroyIcon(hicon);
        
        conversion_result
    }
}

/// Extrai icone REAL de um drive (C:\, D:\, etc.).

/// 
/// DIFERENÃ‡A: Usa path REAL (nÃ£o dummy) e SEM SHGFI_USEFILEATTRIBUTES.
/// Isso forÃ§a o Windows a retornar o Ã­cone especÃ­fico do drive (HD, SSD, USB, etc.).
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
        
        // FLAGS: Sem USEFILEATTRIBUTES - queremos Ã­cone REAL do volume
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

// Helper functions for preview pane
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

fn format_date(timestamp: u64) -> String {
    if timestamp == 0 {
        return "Desconhecido".to_string();
    }
    
    use std::time::{UNIX_EPOCH, Duration, SystemTime};
    
    let datetime = UNIX_EPOCH + Duration::from_secs(timestamp);
    
    // Simple formatting (ideally use chrono)
    if let Ok(duration) = SystemTime::now().duration_since(datetime) {
        let days = duration.as_secs() / 86400;
        if days == 0 {
            "Hoje".to_string()
        } else if days == 1 {
            "Ontem".to_string()
        } else if days < 7 {
            format!("{} dias atras", days)
        } else {
            format!("{} semanas atras", days / 7)
        }
    } else {
        "Futuro".to_string()
    }
}

impl ImageViewerApp {
    /// Filtra itens baseado na query de busca
    fn filter_items(&mut self) {
        if self.search_query.is_empty() {
            self.items = self.all_items.clone();
        } else {
            let query = self.search_query.to_lowercase();
            self.items = self.all_items.iter()
                .filter(|item| item.name.to_lowercase().contains(&query))
                .cloned()
                .collect();
        }
        self.total_items = self.items.len();
    }
    
    /// Ordena itens baseado no modo atual (mantÃ©m pastas sempre primeiro)
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
            
            // 3. Inverte se descending estÃ¡ ativo
            if self.sort_descending {
                ordering.reverse()
            } else {
                ordering
            }
        });
    }
    
    /// Requisita scan assÃ­ncrono de uma pasta para descobrir primeira imagem.
    /// OTIMIZADO: Envia mensagem para worker Ãºnico (zero overhead de threads)
    fn request_folder_scan(&self, folder_path: PathBuf) {
        // Apenas envia para fila - worker processa em background
        let _ = self.cover_worker_sender.send(folder_path);
    }
    
    fn load_folder(&mut self) {
        // 1. Limpeza de Estado (UI Thread)
        self.items.clear();
        self.all_items.clear();  // Limpa backup mestre tambÃ©m
        self.texture_cache.clear();
        self.loading_set.clear();
        self.scanned_folders.clear();
        self.selected_item = None;
        self.is_loading_folder = true;
        self.total_items = 0;
        
        let current_path = self.current_path.clone();
        let file_entry_sender = self.file_entry_sender.clone();
        
        // STREAMING BATCH LOADING: Envia lotes de 250 itens progressivamente
        std::thread::spawn(move || {
            // Buffer para envio em lotes
            let mut batch = Vec::with_capacity(250);
            
            // Prepara busca Win32
            let search_path = if current_path.ends_with('\\') {
                format!("{}*", current_path)
            } else {
                format!("{}\\*", current_path)
            };
            let wide_path: Vec<u16> = search_path.encode_utf16().chain(std::iter::once(0)).collect();
            let mut find_data = WIN32_FIND_DATAW::default();

            unsafe {
                if let Ok(handle) = FindFirstFileW(PCWSTR(wide_path.as_ptr()), &mut find_data) {
                    loop {
                        let len = find_data.cFileName.iter().position(|&c| c == 0).unwrap_or(find_data.cFileName.len());
                        let filename = std::ffi::OsString::from_wide(&find_data.cFileName[0..len])
                            .to_string_lossy()
                            .into_owned();

                        if filename != "." && filename != ".." {
                            let attrs = find_data.dwFileAttributes;
                            
                            // Filtros: hidden/system files
                            let is_hidden = (attrs & FILE_ATTRIBUTE_HIDDEN.0) != 0;
                            let is_system = (attrs & FILE_ATTRIBUTE_SYSTEM.0) != 0;
                            let is_special = matches!(filename.to_lowercase().as_str(),
                                "desktop.ini" | "thumbs.db" | "$recycle.bin" | "system volume information"
                            );
                            
                            if !is_hidden && !is_system && !is_special && !filename.starts_with('.') {
                                let is_dir = (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0;
                                let full_path = PathBuf::from(&current_path).join(&filename);

                                let size = if is_dir { 
                                    0 
                                } else {
                                    ((find_data.nFileSizeHigh as u64) << 32) | (find_data.nFileSizeLow as u64)
                                };

                                let ft = find_data.ftLastWriteTime;
                                let windows_ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
                                let modified = if windows_ticks > 116444736000000000 {
                                    (windows_ticks - 116444736000000000) / 10_000_000
                                } else {
                                    0
                                };

                                let entry = FileEntry {
                                    path: full_path,
                                    name: filename,
                                    is_dir,
                                    size,
                                    modified,
                                    folder_cover: None,  // Lazy load
                                };

                                // Adiciona ao lote
                                batch.push(entry);

                                // SE o lote encheu (250 itens), envia e limpa
                                if batch.len() >= 250 {
                                    let _ = file_entry_sender.send(batch.clone());
                                    batch.clear();
                                }
                            }
                        }

                        if FindNextFileW(handle, &mut find_data).is_err() {
                            break;
                        }
                    }
                    let _ = FindClose(handle);
                }
            }

            // Envia o restante (Ãºltimo lote) se sobrou algo
            if !batch.is_empty() {
                let _ = file_entry_sender.send(batch);
            }
            
            // Envia vetor VAZIO para sinalizar FIM do carregamento
            let _ = file_entry_sender.send(Vec::new());
        });
    }
    
    /// Navega para um caminho, adicionando ao histÃ³rico (corta histÃ³rico futuro)
    fn navigate_to(&mut self, path: &str) {
        // Se jÃ¡ estamos nesse caminho, nÃ£o faz nada
        if self.current_path == path {
            return;
        }
        
        // Corta histÃ³rico "futuro" (se voltamos e navegamos para outro lugar)
        if self.history_index < self.navigation_history.len().saturating_sub(1) {
            self.navigation_history.truncate(self.history_index + 1);
        }
        
        // Adiciona novo caminho ao histÃ³rico
        self.navigation_history.push(path.to_string());
        self.history_index = self.navigation_history.len() - 1;
        
        self.current_path = path.to_string();
        self.path_input = path.to_string();
        self.load_folder();
    }
    
    /// Volta no histÃ³rico (sem adicionar ao histÃ³rico)
    fn go_back(&mut self) {
        if self.can_go_back() {
            self.history_index -= 1;
            self.current_path = self.navigation_history[self.history_index].clone();
            self.path_input = self.current_path.clone();
            self.load_folder();
        }
    }
    
    /// AvanÃ§a no histÃ³rico
    fn go_forward(&mut self) {
        if self.can_go_forward() {
            self.history_index += 1;
            self.current_path = self.navigation_history[self.history_index].clone();
            self.path_input = self.current_path.clone();
            self.load_folder();
        }
    }
    
    /// Sobe um nÃ­vel (adiciona ao histÃ³rico)
    fn go_up_one_level(&mut self) {
        if let Some(parent) = Path::new(&self.current_path).parent() {
            let parent_str = parent.to_string_lossy().to_string();
            if !parent_str.is_empty() {
                self.navigate_to(&parent_str);
            }
        }
    }
    
    /// Pode voltar no histÃ³rico?
    fn can_go_back(&self) -> bool {
        self.history_index > 0
    }
    
    /// Pode avanÃ§ar no histÃ³rico?
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
    
    /// Retorna icone para um arquivo, carregando sob demanda.
    /// Executaveis (.exe, .lnk, .ico) sao cacheados por path completo.
    /// Demais extensoes sao cacheadas por tipo.
    fn get_or_load_icon(
        &mut self, 
        ctx: &egui::Context,
        path: &Path,
    ) -> Option<egui::TextureHandle> {
        let extension = path.extension()?.to_str()?.to_lowercase();
        
        // Decide cache key: path completo para executaveis, extensao para demais
        let cache_key = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            // Cache por path completo - cada executavel tem icone unico
            path.to_string_lossy().to_string()
        } else {
            // Cache por extensao - todos .txt compartilham icone
            format!(".{}", extension)
        };
        
        // Cache hit? Clone do handle (barato)
        if let Some(texture) = self.icon_cache.get(&cache_key) {
            return Some(texture.clone());
        }
        
        // Cache miss -> carrega icone
        let thumbnail_size = self.thumbnail_size;
        let icon_size = if thumbnail_size < 100.0 {
            IconSize::Small
        } else {
            IconSize::Large
        };
        
        // Para executaveis, usa path real; para demais, usa extensao dummy
        let icon_result = if matches!(extension.as_str(), "exe" | "lnk" | "ico") {
            extract_file_icon_by_path(path, icon_size)
        } else {
            extract_file_icon(&format!(".{}", extension), icon_size)
        };
        
        match icon_result {
            Ok((rgba_data, width, height)) => {
                let texture = ctx.load_texture(
                    format!("icon_{}", cache_key),
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &rgba_data,
                    ),
                    egui::TextureOptions::LINEAR,
                );
                
                let cloned = texture.clone();
                self.icon_cache.put(cache_key, texture);
                Some(cloned)
            }
            Err(_) => None,  // Fallback: sem icone
        }
    }
    
    /// Garante que Ã­cone de pasta estÃ¡ carregado.
    fn ensure_folder_icon(&mut self, ctx: &egui::Context) {
        if self.folder_icon_texture.is_some() {
            return; // JÃ¡ carregado
        }
        
        // Windows usa Ã­cone especial para pastas
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
                // Fallback: mantÃ©m emoji
            }
        }
    }
    
    /// Garante que Ã­cone de "Este Computador" estÃ¡ carregado.
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
        // 1. STREAMING: Recebe lotes incrementais de FileEntry
        while let Ok(new_batch) = self.file_entry_receiver.try_recv() {
            if new_batch.is_empty() {
                // Lote vazio = Sinal de "Fim do Carregamento" da thread
                self.is_loading_folder = false;
                // OrdenaÃ§Ã£o final para garantir tudo correto
                self.sort_items();
            } else {
                // Chegou dados! Adiciona Ã  lista mestre
                self.all_items.extend(new_batch);
                
                // Reaplica filtro e ordenaÃ§Ã£o incrementalmente
                self.filter_items(); 
                self.sort_items();
            }
            ctx.request_repaint();
        }
        
        // 2. Cover Worker: Recebe resultados de capas de pasta
        let mut folder_updates = false;
        while let Ok((folder_path, cover_opt)) = self.cover_worker_receiver.try_recv() {
            if let Some(cover) = cover_opt {
                // Atualiza em items (lista filtrada/ordenada)
                if let Some(item) = self.items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover.clone());
                    
                    // JÃ¡ requisita thumbnail da imagem encontrada
                    if !self.texture_cache.contains(&cover) && !self.loading_set.contains(&cover) {
                        self.request_thumbnail_load(cover.clone());
                    }
                    folder_updates = true;
                }
                // TambÃ©m atualiza em all_items (persistÃªncia atravÃ©s de filtros)
                if let Some(item) = self.all_items.iter_mut().find(|i| i.path == folder_path) {
                    item.folder_cover = Some(cover);
                }
            }
        }
        if folder_updates {
            ctx.request_repaint();
        }
        
        // 3. Individual thumbnails
        let mut received_any = false;
        let mut _new_items_added = false;
        
        while let Ok(thumbnail_data) = self.image_receiver.try_recv() {
            received_any = true;
            
            // SÃ³ processa thumbnails (image_data nÃ£o vazio)
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
    
    // --- DETALHES (LIST VIEW) ---
    fn render_list_view(&mut self, ui: &mut egui::Ui) {
        let row_height = 24.0;
        let available_w = ui.available_width();
        
        // Larguras das colunas
        let w_name = (available_w - 380.0).max(200.0);
        let w_date = 140.0;
        let w_type = 120.0;
        let w_size = 100.0;
        
        // CabeÃ§alho da Tabela
        ui.horizontal(|ui| {
            ui.style_mut().spacing.item_spacing.x = 0.0;
            
            let mut draw_header = |ui: &mut egui::Ui, text: &str, width: f32, mode: SortMode| {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::click());
                let is_active = self.sort_mode == mode;
                
                if ui.is_rect_visible(rect) {
                    if is_active {
                        ui.painter().rect_filled(rect, 2.0, egui::Color32::from_gray(230));
                    }
                    let text_color = if is_active { egui::Color32::BLACK } else { egui::Color32::from_gray(100) };
                    ui.painter().text(
                        rect.min + egui::vec2(8.0, 4.0),
                        egui::Align2::LEFT_TOP,
                        text,
                        egui::FontId::proportional(12.0),
                        text_color,
                    );
                    if is_active {
                        let arrow = if self.sort_descending { "v" } else { "^" };
                        ui.painter().text(
                            rect.max - egui::vec2(15.0, 8.0),
                            egui::Align2::CENTER_CENTER,
                            arrow,
                            egui::FontId::proportional(10.0),
                            text_color,
                        );
                    }
                }
                
                if response.clicked() {
                    if self.sort_mode == mode {
                        self.sort_descending = !self.sort_descending;
                    } else {
                        self.sort_mode = mode;
                        self.sort_descending = false;
                    }
                    self.sort_items();
                }
            };

            draw_header(ui, "Nome", w_name, SortMode::Name);
            draw_header(ui, "Data", w_date, SortMode::Date);
            draw_header(ui, "Tipo", w_type, SortMode::Name); // Tipo usa Name sort secundÃ¡rio
            draw_header(ui, "Tamanho", w_size, SortMode::Size);
        });
        
        ui.separator();

        // Lista Virtualizada
        let total_rows = self.items.len();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
            ui,
            row_height + 2.0,
            total_rows,
            |ui, row_range| {
                for i in row_range {
                    if i >= self.items.len() { break; }
                    let item = self.items[i].clone();
                    let is_selected = self.selected_item == Some(i);

                    ui.push_id(i, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), row_height), 
                            egui::Sense::click()
                        );

                        // SeleÃ§Ã£o e AÃ§Ã£o
                        if response.clicked() {
                            self.selected_item = Some(i);
                            self.selected_file = Some(item.clone());
                            
                            // Trigger thumbnail load for sidebar preview
                            if !item.is_dir {
                                if !self.texture_cache.contains(&item.path) && !self.loading_set.contains(&item.path) {
                                    self.request_thumbnail_load(item.path.clone());
                                }
                            }
                        }
                        if response.double_clicked() {
                            if item.is_dir {
                                self.navigate_to(&item.path.to_string_lossy());
                            } else {
                                open_with_shell(&item.path);
                            }
                        }

                        // Background Selection
                        if is_selected {
                            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(205, 232, 255));
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(245));
                        }

                        let text_color = egui::Color32::BLACK;
                        let secondary_color = egui::Color32::from_gray(100);
                        
                        // 1. Icone + Nome
                        let icon_size_px = 16.0;
                        let icon_rect = egui::Rect::from_min_size(
                            rect.min + egui::vec2(4.0, 4.0),
                            egui::vec2(icon_size_px, icon_size_px)
                        );
                        
                        if item.is_dir {
                            // Pasta: icone nativo do Windows
                            self.ensure_folder_icon(ui.ctx());
                            if let Some(folder_icon) = &self.folder_icon_texture {
                                ui.painter().image(
                                    folder_icon.id(),
                                    icon_rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE
                                );
                            } else {
                                ui.painter().text(icon_rect.min, egui::Align2::LEFT_TOP, "[D]", egui::FontId::proportional(14.0), egui::Color32::from_rgb(255, 193, 7));
                            }
                        } else {
                            // Arquivo: tenta carregar icone nativo
                            if let Some(file_icon) = self.get_or_load_icon(ui.ctx(), &item.path) {
                                ui.painter().image(
                                    file_icon.id(),
                                    icon_rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::WHITE
                                );
                            } else {
                                ui.painter().text(icon_rect.min, egui::Align2::LEFT_TOP, "[F]", egui::FontId::proportional(14.0), egui::Color32::GRAY);
                            }
                        }

                        // Nome (truncado para caber na coluna - safe UTF-8)
                        let max_name_chars = ((w_name - 30.0) / 7.0) as usize;
                        let display_name: String = if item.name.chars().count() > max_name_chars && max_name_chars > 3 {
                            let truncated: String = item.name.chars().take(max_name_chars.saturating_sub(3)).collect();
                            format!("{}...", truncated)
                        } else {
                            item.name.clone()
                        };
                        ui.painter().text(
                            rect.min + egui::vec2(24.0, 5.0),
                            egui::Align2::LEFT_TOP,
                            display_name,
                            egui::FontId::proportional(12.0),
                            text_color,
                        );

                        // 2. Data
                        ui.painter().text(
                            egui::pos2(rect.min.x + w_name, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            format_date(item.modified),
                            egui::FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 3. Tipo (truncado)
                        let type_str = get_file_type_string(&item);
                        let max_type_chars = 14; // ~100px at 7px per char
                        let display_type: String = if type_str.chars().count() > max_type_chars {
                            type_str.chars().take(max_type_chars - 2).collect::<String>() + ".."
                        } else {
                            type_str
                        };
                        ui.painter().text(
                            egui::pos2(rect.min.x + w_name + w_date, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            display_type,
                            egui::FontId::proportional(12.0),
                            secondary_color,
                        );

                        // 4. Tamanho
                        let size_str = if item.is_dir { "".to_string() } else { format_size(item.size) };
                        ui.painter().text(
                            egui::pos2(rect.min.x + w_name + w_date + w_type, rect.min.y + 5.0),
                            egui::Align2::LEFT_TOP,
                            size_str,
                            egui::FontId::proportional(12.0),
                            secondary_color,
                        );
                    });
                }
            }
        );
    }

    // --- GRANDE (GRID VIEW) ---
    fn render_grid_view(&mut self, ui: &mut egui::Ui) {
        let padding = 8.0;
        let item_w = self.thumbnail_size;
        let item_h = self.thumbnail_size + 20.0;  // Altura: thumb + texto
        let available_w = ui.available_width();
        let cols = ((available_w - padding) / (item_w + padding)).floor().max(1.0) as usize;
        self.last_grid_cols = cols;
        
        // NavegaÃ§Ã£o Teclado
        if ui.input(|i| i.focused) {
            let current_index = self.items.iter().position(|x| self.selected_file.as_ref().map_or(false, |f| f.path == x.path));
            let mut new_index: Option<usize> = None;
            
            if ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) { 
                new_index = current_index.map(|idx| idx.saturating_add(1)).or(Some(0)); 
            }
            else if ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) { 
                new_index = current_index.map(|idx| idx.saturating_sub(1)); 
            }
            else if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) { 
                new_index = current_index.map(|idx| idx + cols).or(Some(0)); 
            }
            else if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) { 
                new_index = current_index.map(|idx| idx.saturating_sub(cols)); 
            }

            if let Some(idx) = new_index {
                let clamped = idx.min(self.items.len().saturating_sub(1));
                if let Some(item) = self.items.get(clamped) {
                    self.selected_file = Some(item.clone());
                    self.selected_item = Some(clamped);
                }
            }
            
            // Enter para abrir
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(selected) = &self.selected_file.clone() {
                    if selected.is_dir {
                        self.navigate_to(&selected.path.to_string_lossy());
                    } else {
                        open_with_shell(&selected.path);
                    }
                }
            }
        }

        // Grid Virtualizado
        let count = self.items.len();
        let rows = (count as f32 / cols as f32).ceil() as usize;
        let total_height = rows as f32 * (item_h + padding) + padding;
        
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let content_min = ui.min_rect().min;
            ui.allocate_rect(egui::Rect::from_min_size(content_min, egui::vec2(available_w, total_height)), egui::Sense::hover());
            
            let clip_rect = ui.clip_rect();
            let start_y = (clip_rect.top() - content_min.y).max(0.0);
            let end_y = start_y + clip_rect.height();
            
            let visible_min_row = (start_y / (item_h + padding)).floor() as usize;
            let visible_max_row = ((end_y / (item_h + padding)).ceil() as usize + 1).min(rows);
            
            let loop_min_row = visible_min_row.saturating_sub(2);
            let loop_max_row = (visible_max_row + 2).min(rows);
            
            for row in loop_min_row..loop_max_row {
                for col in 0..cols {
                    let index = row * cols + col;
                    if index >= count { break; }
                    
                    let x_pos = col as f32 * (item_w + padding) + padding;
                    let y_pos = row as f32 * (item_h + padding) + padding;
                    let rect = egui::Rect::from_min_size(content_min + egui::vec2(x_pos, y_pos), egui::vec2(item_w, item_h));
                    
                    if ui.is_rect_visible(rect) {
                        let response = ui.interact(rect, ui.id().with(index), egui::Sense::click());
                        if response.clicked() {
                            self.selected_file = Some(self.items[index].clone());
                            self.selected_item = Some(index);
                        }
                        if response.double_clicked() {
                            let item = self.items[index].clone();
                            if item.is_dir { self.navigate_to(&item.path.to_string_lossy()); }
                            else { open_with_shell(&item.path); }
                        }
                        
                        if self.selected_item == Some(index) {
                            ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 120, 215)), egui::StrokeKind::Inside);
                            ui.painter().rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(0, 120, 215, 30));
                        }
                        
                        // Content area with margin for selection border visibility
                        let content_margin = 3.0;
                        let inner_rect = rect.shrink(content_margin);
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                            self.render_item_slot(ui, index);
                        });
                    }
                }
            }
        });
    }

    fn render_item_slot(&mut self, ui: &mut egui::Ui, idx: usize) {
        if idx >= self.items.len() {
            return;
        }
        
        let item = self.items[idx].clone();  // Clone para evitar borrow conflicts
        let _is_selected = self.selected_item == Some(idx);
        
        // ==== DIRECTORY RENDERING ====
        if item.is_dir {
            // --- GATILHO LAZY LOAD ---
            // Se nÃ£o tem capa E ainda nÃ£o foi escaneado: Dispara Scan.
            if item.folder_cover.is_none() && !self.scanned_folders.contains(&item.path) {
                self.scanned_folders.insert(item.path.clone());
                self.request_folder_scan(item.path.clone());
            }
            
            // GEOMETRIA
            let available_h = ui.available_height();
            let folder_w = self.thumbnail_size * 0.60;
            let folder_h = folder_w * 0.85;
            let text_height = 18.0;
            let content_h = folder_h + text_height;
            let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
            
            // Margem superior para centralizar verticalmente
            ui.add_space(vertical_margin);
            
            // Centraliza a pasta horizontalmente na celula
            let cell_width = ui.available_width();
            let x_offset = (cell_width - folder_w) / 2.0;
            let start_pos = ui.cursor().min + egui::vec2(x_offset.max(0.0), 0.0);
            let folder_rect = egui::Rect::from_min_size(start_pos, egui::vec2(folder_w, folder_h));

            // CORES
            let color_back = egui::Color32::from_rgb(200, 160, 50);
            let color_front = egui::Color32::from_rgb(255, 210, 70);

            // DimensÃµes
            let tab_h = folder_h * 0.15;
            let tab_w = folder_w * 0.40;
            let front_h = folder_h * 0.50;

            // === DESENHO 1: BASE SÃ“LIDA (evita qualquer gap) ===
            // Desenha TODO o corpo como uma Ãºnica forma sÃ³lida
            ui.painter().rect_filled(
                egui::Rect::from_min_size(folder_rect.min, egui::vec2(tab_w, tab_h)),
                egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 },
                color_back
            );
            ui.painter().rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(folder_rect.min.x, folder_rect.min.y + tab_h),
                    folder_rect.max
                ),
                egui::CornerRadius { nw: 0, ne: 3, sw: 4, se: 4 },
                color_back
            );

            // === DESENHO 2: PREVIEW (com clipping para nÃ£o escapar) ===
            if let Some(cover_path) = &item.folder_cover {
                if !self.texture_cache.contains(cover_path) && !self.loading_set.contains(cover_path) {
                    if self.loading_set.len() < MAX_CONCURRENT_LOADS {
                        self.loading_set.insert(cover_path.clone());
                        self.request_thumbnail_load(cover_path.clone());
                    }
                }
            }

            if let Some(tex) = item.folder_cover.as_ref().and_then(|p| self.texture_cache.get(p)) {
                // Ãrea onde o preview pode aparecer (com margens)
                let margin_x = 6.0;
                let margin_top = 4.0;
                let preview_area = egui::Rect::from_min_max(
                    egui::pos2(folder_rect.min.x + margin_x, folder_rect.min.y + tab_h + margin_top),
                    egui::pos2(folder_rect.max.x - margin_x, folder_rect.max.y - front_h)
                );

                let size = tex.size();
                let tex_size = egui::vec2(size[0] as f32, size[1] as f32);
                let aspect_img = tex_size.x / tex_size.y;
                let aspect_view = preview_area.width() / preview_area.height();

                let uv_rect = if aspect_img > aspect_view {
                    let scale = aspect_view / aspect_img;
                    let offset = (1.0 - scale) / 2.0;
                    egui::Rect::from_min_max(egui::pos2(offset, 0.0), egui::pos2(1.0 - offset, 1.0))
                } else {
                    let scale = aspect_img / aspect_view;
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, scale))
                };

                // Usa push_clip_rect para garantir que a imagem nÃ£o escape
                ui.painter().with_clip_rect(preview_area).image(tex.id(), preview_area, uv_rect, egui::Color32::WHITE);
            }

            // === DESENHO 3: BOLSO FRONTAL (sobrepÃµe preview) ===
            let front_rect = egui::Rect::from_min_max(
                egui::pos2(folder_rect.min.x, folder_rect.max.y - front_h),
                folder_rect.max
            );
            ui.painter().rect_filled(front_rect, egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 }, color_front);

            // Borda sutil
            ui.painter().rect_stroke(
                front_rect,
                egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 },
                egui::Stroke::new(1.0, egui::Color32::from_rgb(200, 150, 30)),
                egui::StrokeKind::Inside
            );

            // Aloca espaÃ§o da pasta
            ui.allocate_rect(folder_rect, egui::Sense::hover());

            // TEXTO: Usa Label com truncate (igual aos arquivos) para respeitar limites
            ui.add_space(6.0);  // Gap entre pasta e texto
            ui.vertical_centered(|ui| {
                ui.add(egui::Label::new(
                    egui::RichText::new(&item.name)
                        .size(11.0)
                        .color(egui::Color32::BLACK)
                ).truncate());
            });
        }
            // ==== FILE RENDERING ====
            else {
                let path_clone = item.path.clone();
                let is_selected = self.selected_item == Some(idx);
                
                // Detecta se e arquivo de midia
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
                
                // Thumbnail loading para arquivos de midia
                if is_media_file {
                    let has_texture = self.texture_cache.contains(&path_clone);
                    let is_loading = self.loading_set.contains(&path_clone);
                    
                    if !has_texture && !is_loading && self.loading_set.len() < MAX_CONCURRENT_LOADS {
                        self.loading_set.insert(path_clone.clone());
                        self.request_thumbnail_load(path_clone.clone());
                    }
                }
                
                // Carrega icone (sempre, servira como fallback)
                let file_icon = self.get_or_load_icon(ui.ctx(), &path_clone);
                
                // GEOMETRIA - reduz tamanho para caber na area com margem
                let available_h = ui.available_height();
                let available_w = ui.available_width();
                let thumb_size = (self.thumbnail_size - 6.0).min(available_w - 4.0); // 6px margem total
                let text_height = 18.0;
                let content_h = thumb_size + text_height;
                let vertical_margin = ((available_h - content_h) / 2.0).max(2.0);
                
                // Margem superior para centralizar verticalmente
                ui.add_space(vertical_margin);
                
                // Centraliza horizontalmente na area disponivel
                let x_offset = (available_w - thumb_size) / 2.0;
                let start_pos = ui.cursor().min + egui::vec2(x_offset.max(0.0), 0.0);
                let thumb_rect = egui::Rect::from_min_size(start_pos, egui::vec2(thumb_size, thumb_size));
                
                // Desenha thumbnail ou icone
                let mut drew_something = false;
                if is_media_file {
                    if let Some(texture) = self.texture_cache.get(&path_clone) {
                        // Thumbnail carregado - mantem aspect ratio
                        let tex_size = texture.size_vec2();
                        let aspect = tex_size.x / tex_size.y;
                        let (draw_w, draw_h) = if aspect > 1.0 {
                            (thumb_size, thumb_size / aspect)
                        } else {
                            (thumb_size * aspect, thumb_size)
                        };
                        let offset_x = (thumb_size - draw_w) / 2.0;
                        let offset_y = (thumb_size - draw_h) / 2.0;
                        let draw_rect = egui::Rect::from_min_size(
                            thumb_rect.min + egui::vec2(offset_x, offset_y),
                            egui::vec2(draw_w, draw_h)
                        );
                        ui.painter().image(texture.id(), draw_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        drew_something = true;
                    }
                }

                if !drew_something {
                    // Fallback para icone do Windows ou placeholder
                    ui.painter().rect_filled(thumb_rect, 4.0, egui::Color32::from_gray(248));
                    if let Some(icon_texture) = file_icon {
                        let icon_size = thumb_size * 0.5;
                        let icon_rect = egui::Rect::from_center_size(thumb_rect.center(), egui::vec2(icon_size, icon_size));
                        ui.painter().image(icon_texture.id(), icon_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    } else {
                        // Se nem o icone carregou, mostra "..." se for midia ou "[F]" se nao
                        let text = if is_media_file { "..." } else { "[F]" };
                        ui.painter().text(thumb_rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(thumb_size * 0.3), egui::Color32::GRAY);
                    }
                }
                
                // Aloca espaco do thumbnail
                ui.allocate_rect(thumb_rect, egui::Sense::hover());
                
                // Texto do nome - igual as pastas
                ui.add_space(4.0);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Label::new(
                        egui::RichText::new(&item.name)
                            .size(11.0)
                            .color(egui::Color32::BLACK)
                    ).truncate());
                });
            }
        }
    }


impl eframe::App for ImageViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_incoming_messages(ctx);
        self.ensure_folder_icon(ctx);
        self.ensure_computer_icon(ctx);  // Carrega Ã­cone "Este Computador"

        
        // Windows 11 style sidebar
        // Left Sidebar moved to after TopPanels for correct layout

        
        // Top navigation bar
        egui::TopBottomPanel::top("nav_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // BotÃ£o Voltar (desabilitado se nÃ£o pode voltar)
                let can_back = self.can_go_back();
                if ui.add_enabled(can_back, egui::Button::new("<")).clicked() {
                    self.go_back();
                }
                
                // BotÃ£o AvanÃ§ar (desabilitado se nÃ£o pode avanÃ§ar)
                let can_forward = self.can_go_forward();
                if ui.add_enabled(can_forward, egui::Button::new(">")).clicked() {
                    self.go_forward();
                }
                
                // BotÃ£o Subir
                if ui.button("^").clicked() {
                    self.go_up_one_level();
                }
                
                ui.separator();
                
                // Search field (NEW)
                // Search field
                let search_response = ui.add_sized(
                    egui::vec2(150.0, 20.0),
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Buscar...")
                );
                if search_response.changed() {
                    self.filter_items();
                    self.sort_items();
                }
                
                ui.separator();
                
                // Sorting controls (BEFORE address bar to ensure visibility)
                ui.label("Ordenar:");
                egui::ComboBox::from_id_salt("sort_mode")
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
                let sort_icon = if self.sort_descending { "v" } else { "^" };
                if ui.button(sort_icon).clicked() {
                    self.sort_descending = !self.sort_descending;
                    self.sort_items();
                }

                ui.separator();
                
                // VIEW MODE (Grid vs List)
                ui.selectable_value(&mut self.view_mode, ViewMode::Grid, "[#] Grade");
                ui.selectable_value(&mut self.view_mode, ViewMode::List, "[=] Lista");
                
                ui.separator();
                
                // Toggle preview pane
                if ui.button("[*]").on_hover_text("Preview").clicked() {
                    self.show_preview_panel = !self.show_preview_panel;
                }
                
                ui.separator();
                
                // Barra de endereÃ§o editÃ¡vel
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
                        // Restaura o path atual se invÃ¡lido
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
                
            });
        });
        
        // Windows 11 style sidebar (Restored)
        egui::SidePanel::left("sidebar")
            .min_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(10.0);
                
                // Header "Este Computador" com Ã­cone nativo
                ui.horizontal(|ui| {
                    if let Some(icon) = &self.computer_icon {
                        ui.add(egui::Image::new(icon)
                            .max_size(egui::vec2(16.0, 16.0))
                            .maintain_aspect_ratio(true));
                    }
                    ui.label(egui::RichText::new("Este Computador").strong().size(16.0));
                });
                
                ui.separator();
                
                ui.add_space(5.0);
                
                for (disk_path, disk_label) in &self.disks.clone() {
                    // PrÃ©-carrega Ã­cone do drive se nÃ£o estiver no cache
                    let drive_icon = if let Some(icon) = self.drive_icon_cache.get(disk_path) {
                        Some(icon.clone())
                    } else {
                        // Tenta carregar Ã­cone real do drive
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
                    
                    
                    // Renderiza drive com Ã­cone + label usando interact() para controle total do cursor
                    let is_selected = self.current_path.starts_with(disk_path);
                    
                    // Desenha conteÃºdo no horizontal layout
                    let (mut rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 24.0),
                        egui::Sense::click()  // Captura cliques, sem texto selecionÃ¡vel
                    );
                    
                    // Expande rect para preencher toda a largura da sidebar (remove gaps)
                    rect.min.x = ui.clip_rect().min.x;
                    rect.max.x = ui.clip_rect().max.x;
                    
                    // SÃ³ desenha se visÃ­vel
                    if ui.is_rect_visible(rect) {
                        // Background de seleÃ§Ã£o
                        if is_selected {
                            ui.painter().rect_filled(
                                rect,
                                0.0,  // Sem cantos arredondados para ficar flush com as bordas
                                egui::Color32::from_rgb(200, 220, 240)
                            );
                        }
                        
                        // Hover effect
                        if response.hovered() && !is_selected {
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                egui::Color32::from_rgba_unmultiplied(200, 220, 240, 50)
                            );
                        }
                        
                        // Desenha Ã­cone e texto manualmente
                        let mut cursor_x = rect.min.x + 5.0;
                        
                        // Ãcone
                        if let Some(icon) = drive_icon {
                            let icon_rect = egui::Rect::from_min_size(
                                egui::pos2(cursor_x, rect.center().y - 8.0),
                                egui::vec2(16.0, 16.0)
                            );
                            ui.painter().image(icon.id(), icon_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                            cursor_x += 20.0;
                        } else {
                            ui.painter().text(
                                egui::pos2(cursor_x, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                "ðŸ’¾",
                                egui::FontId::proportional(14.0),
                                ui.visuals().text_color()
                            );
                            cursor_x += 20.0;
                        }
                        
                        // Texto
                        ui.painter().text(
                            egui::pos2(cursor_x, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            disk_label,
                            egui::FontId::proportional(14.0),
                            if is_selected { 
                                egui::Color32::from_rgb(0, 50, 100) 
                            } else { 
                                ui.visuals().text_color() 
                            }
                        );
                    }
                    
                    if response.clicked() {
                        self.navigate_to(disk_path);
                    }
                    
                    
                    ui.add_space(3.0);
                }
            });
        

        
        // Preview Pane (Windows Explorer style) - ANTES do CentralPanel
        if self.show_preview_panel {
            egui::SidePanel::right("preview_panel")
                .resizable(true)
                .default_width(300.0)
                .min_width(250.0)
                .max_width(500.0)
                .show(ctx, |ui| {
                    if let Some(file) = self.selected_file.clone() {
                        ui.heading("Detalhes");
                        ui.separator();
                        
                        // Preview de imagem/video (se houver thumbnail)
                        let has_thumbnail = self.texture_cache.peek(&file.path).is_some();
                        let is_media = file.path.extension()
                            .and_then(|e| e.to_str())
                            .map(|ext| {
                                let ext_lower = ext.to_lowercase();
                                matches!(ext_lower.as_str(),
                                    "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
                                    "tiff" | "tif" | "ico" | "heic" | "heif" | "avif" |
                                    "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" |
                                    "webm" | "m4v" | "mpg" | "mpeg" | "3gp" | "ts"
                                )
                            })
                            .unwrap_or(false);
                        
                        let texture = self.texture_cache.peek(&file.path).cloned();

                        if let (Some(tex), true) = (texture, is_media) {
                            // Mostra thumbnail de imagem/video
                            let max_preview_width = ui.available_width() - 20.0;
                            let max_preview_size = egui::vec2(max_preview_width, max_preview_width);
                            
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Image::new(&tex)
                                    .max_size(max_preview_size)
                                    .fit_to_original_size(1.0)
                                    .shrink_to_fit());
                            });
                            ui.separator();
                        } else if !file.is_dir {
                            // Arquivo sem thumbnail -> mostra icone do Windows
                            // Aqui o self.get_or_load_icon pode ser chamado porque 'file' eh um clone
                            if let Some(icon) = self.get_or_load_icon(ui.ctx(), &file.path) {
                                let icon_display_size = 64.0;
                                ui.vertical_centered(|ui| {
                                    ui.add_space(20.0);
                                    ui.add(egui::Image::new(&icon)
                                        .max_size(egui::vec2(icon_display_size, icon_display_size))
                                        .maintain_aspect_ratio(true));
                                    ui.add_space(20.0);
                                });
                                ui.separator();
                            }
                        }
                        
                        // Tabela de detalhes
                        egui::Grid::new("details_grid")
                            .num_columns(2)
                            .spacing([10.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Nome:");
                                ui.add(egui::Label::new(&file.name)
                                    .wrap()
                                    .truncate());
                                ui.end_row();
                                
                                ui.label("Tamanho:");
                                ui.label(format_size(file.size));
                                ui.end_row();
                                
                                ui.label("Tipo:");
                                if file.is_dir {
                                    ui.label("Pasta");
                                } else {
                                    let ext = file.path.extension()
                                        .and_then(|e| e.to_str())
                                        .unwrap_or("Arquivo");
                                    ui.label(ext.to_uppercase());
                                }
                                ui.end_row();
                                
                                ui.label("Data:");
                                ui.label(format_date(file.modified));
                                ui.end_row();
                            });
                    } else {
                        ui.vertical_centered(|ui| {
                            ui.add_space(100.0);
                            ui.label("Selecione um arquivo");
                            ui.label("para ver detalhes");
                        });
                    }
                });
        }
        
        // Central Panel
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.is_loading_folder && self.items.is_empty() {
                ui.centered_and_justified(|ui| { 
                    ui.spinner(); 
                    ui.label("Carregando...");
                });
            } else if self.items.is_empty() {
                ui.centered_and_justified(|ui| { ui.label("Pasta vazia"); });
            } else {
                match self.view_mode {
                    ViewMode::Grid => self.render_grid_view(ui),
                    ViewMode::List => self.render_list_view(ui),
                }

                // Spinner pequeno no canto se ainda carregando
                if self.is_loading_folder {
                    let rect = ui.max_rect();
                    let spinner_rect = egui::Rect::from_min_size(
                        rect.right_bottom() - egui::vec2(24.0, 24.0),
                        egui::vec2(16.0, 16.0)
                    );
                    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(spinner_rect), |ui| {
                        ui.spinner();
                    });
                }
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

