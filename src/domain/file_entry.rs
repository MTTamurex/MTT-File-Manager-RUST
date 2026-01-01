use std::path::{Path, PathBuf};

/// Informações de volume/drive para a view "Este Computador"
#[derive(Clone, Debug)]
pub struct DriveInfo {
    pub file_system: String,
    pub total_space: u64,
    pub free_space: u64,
}

/// Entry de arquivo/pasta com metadados cacheados para ordenação
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,      // Cache do nome para sort rápido
    pub is_dir: bool,      // Pastas primeiro
    pub size: u64,         // Tamanho em bytes (0 para diretórios)
    pub modified: u64,     // Timestamp (segundos desde UNIX_EPOCH)
    pub folder_cover: Option<PathBuf>,  // Primeira imagem encontrada na pasta (para preview)
    pub drive_info: Option<DriveInfo>,  // Metadados de drive (opcional)
    pub sync_status: SyncStatus,        // Status de sincronização OneDrive
}

impl FileEntry {
    pub fn from_path(path: PathBuf, is_dir: bool) -> Self {
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
        
        // OTIMIZAÇÃO: Lazy loading - sempre None inicialmente.
        // O scan será disparado por request_folder_scan() quando a pasta ficar visível.
        let folder_cover = None;
        let drive_info = None;
        
        Self { path, name, is_dir, size, modified, folder_cover, drive_info, sync_status: SyncStatus::None }
    }
    
    pub fn path(&self) -> &Path {
        &self.path
    }
    
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Helper para exibir tipo do arquivo na Lista
pub fn get_file_type_string(entry: &FileEntry) -> String {
    if entry.is_dir {
        return "Pasta".to_string();
    }
    if let Some(ext) = entry.path.extension() {
        return format!("Arquivo {}", ext.to_string_lossy().to_uppercase());
    }
    "Arquivo".to_string()
}

/// Busca primeiro item de mídia (imagem ou vídeo) em uma pasta para usar como preview
/// Verifica apenas os primeiros 15 arquivos para performance
pub fn find_folder_preview_item(folder_path: &Path) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(folder_path) {
        for entry in entries.flatten().take(15) {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    match ext.to_lowercase().as_str() {
                        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" |
                        "mp4" | "mkv" | "avi" | "mov" | "webm" => return Some(path),
                        _ => {}
                    }
                }
            }
        }
    }
    None
}

/// Modo de ordenação
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SortMode {
    Name,
    Date,
    Size,
    Type,
}

/// Modo de visualização
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ViewMode {
    Grid,
    List,
}

/// Tamanho de ícones
#[derive(Copy, Clone)]
pub enum IconSize {
    Small,  // 16x16 ou 32x32 (depende do DPI)
    Large,  // 32x32 ou 48x48
    Jumbo,  // 256x256 (via Shell Image Factory)
}

/// Posição das pastas na listagem
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum FoldersPosition {
    First,   // Pastas antes de arquivos (padrão)
    Last,    // Arquivos antes de pastas
    Mixed,   // Misturados por critério de ordenação
}

/// Status de sincronização OneDrive
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SyncStatus {
    #[default]
    None,             // Not a cloud file / Normal
    CloudOnly,        // "Available online" (needs download)
    Pinned,           // "Always keep on this device" (Green check)
    LocallyAvailable, // Downloaded on demand (Green outline)
}
