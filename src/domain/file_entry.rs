use crate::infrastructure::windows::system_info::DriveType;
use std::path::{Path, PathBuf};

/// Informações de volume/drive para a view "Este Computador"
#[derive(Clone, Debug)]
pub struct DriveInfo {
    pub file_system: String,
    pub total_space: u64,
    pub free_space: u64,
    pub drive_type: DriveType, // Tipo do drive (local, rede, removível, etc)
}

/// Entry de arquivo/pasta com metadados cacheados para ordenação
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,                           // Cache do nome para sort rápido
    pub is_dir: bool,                           // Pastas primeiro
    pub size: u64,                              // Tamanho em bytes (0 para diretórios)
    pub modified: u64,                          // Timestamp (segundos desde UNIX_EPOCH)
    pub folder_cover: Option<PathBuf>, // Primeira imagem encontrada na pasta (para preview)
    pub drive_info: Option<DriveInfo>, // Metadados de drive (opcional)
    pub sync_status: SyncStatus,       // Status de sincronização OneDrive
    pub deletion_date: Option<String>, // Data de exclusão (apenas Lixeira)
    pub recycle_original_path: Option<PathBuf>, // Caminho original para restauração (apenas Lixeira)
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
                let modified = m
                    .modified()
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

        Self {
            path,
            name,
            is_dir,
            size,
            modified,
            folder_cover,
            drive_info,
            sync_status: SyncStatus::None,
            deletion_date: None,
            recycle_original_path: None,
        }
    }

    /// PERFORMANCE: Check if this file is a media file (video, audio, or image)
    /// This method computes the value on-demand to avoid storing it in FileEntry
    pub fn is_media(&self) -> bool {
        if self.is_dir {
            return false;
        }
        self.path
            .extension()
            .map(|ext| crate::infrastructure::windows::is_media_extension(&ext.to_string_lossy()))
            .unwrap_or(false)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_zip(&self) -> bool {
        ends_with_ignore_case(&self.name, ".zip")
    }

    pub fn is_archive(&self) -> bool {
        is_archive_extension(&self.name)
    }
}

pub fn ends_with_ignore_case(s: &str, suffix: &str) -> bool {
    if s.len() < suffix.len() {
        return false;
    }
    let start = s.len() - suffix.len();
    s.as_bytes()[start..]
        .iter()
        .zip(suffix.as_bytes())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Extensões de arquivo compactado suportadas para navegação via Windows Shell Namespace.
/// Extensões compostas (.tar.gz) devem vir antes das simples (.gz).
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.zst", ".tzst", ".tar.xz", ".txz", ".tar", ".zip",
    ".7z", ".rar", ".gz", ".gzip",
];

/// Checa se um nome de arquivo termina com uma extensão de arquivo compactado (case-insensitive).
#[inline]
pub fn is_archive_extension(name: &str) -> bool {
    ARCHIVE_EXTENSIONS
        .iter()
        .any(|ext| ends_with_ignore_case(name, ext))
}

/// Checa se um caminho (já em lowercase) passa por dentro de um arquivo compactado.
/// Ex: "C:\arquivo.7z\subdir\file.txt" → true
pub fn path_contains_archive_segment(path_lower: &str) -> bool {
    ARCHIVE_EXTENSIONS.iter().any(|ext| {
        let with_backslash = format!("{}\\", ext);
        let with_fwdslash = format!("{}/", ext);
        path_lower.contains(&with_backslash) || path_lower.contains(&with_fwdslash)
    })
}

/// Retorna o label de tipo para exibição de um arquivo compactado.
/// Ex: "Arquivo ZIP", "Arquivo RAR". Retorna None se não for arquivo compactado.
pub fn archive_type_label(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some("Arquivo TAR.GZ")
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
        Some("Arquivo TAR.BZ2")
    } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") {
        Some("Arquivo TAR.ZST")
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Some("Arquivo TAR.XZ")
    } else if lower.ends_with(".tar") {
        Some("Arquivo TAR")
    } else if lower.ends_with(".zip") {
        Some("Arquivo ZIP")
    } else if lower.ends_with(".7z") {
        Some("Arquivo 7Z")
    } else if lower.ends_with(".rar") {
        Some("Arquivo RAR")
    } else if lower.ends_with(".gz") || lower.ends_with(".gzip") {
        Some("Arquivo GZ")
    } else {
        None
    }
}

/// Helper para exibir tipo do arquivo na Lista
pub fn get_file_type_string(entry: &FileEntry) -> String {
    if let Some(label) = archive_type_label(&entry.name) {
        return label.to_string();
    }
    if entry.is_dir {
        return "Pasta".to_string();
    }
    if let Some(ext) = entry.path.extension() {
        return format!("Arquivo {}", ext.to_string_lossy().to_uppercase());
    }
    "Arquivo".to_string()
}

/// Modo de ordenação
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SortMode {
    Name,
    Date,
    Size,
    Type,
    /// Espaço total do drive (apenas para Computer View)
    DriveTotalSpace,
    /// Espaço livre do drive (apenas para Computer View)
    DriveFreeSpace,
}

/// Modo de visualização
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ViewMode {
    Grid,
    List,
}

/// Tamanho de ícones
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum IconSize {
    Small, // 16x16 ou 32x32 (depende do DPI)
    Large, // 32x32 ou 48x48
    Jumbo, // 256x256 (via Shell Image Factory)
}

/// Posição das pastas na listagem
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum FoldersPosition {
    First, // Pastas antes de arquivos (padrão)
    Last,  // Arquivos antes de pastas
    Mixed, // Misturados por critério de ordenação
}

/// Status de sincronização OneDrive
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SyncStatus {
    #[default]
    None, // Not a cloud file / Normal
    CloudOnly,        // "Available online" (needs download)
    Syncing,          // Currently syncing (blue arrows)
    Pinned,           // "Always keep on this device" (Green check)
    LocallyAvailable, // Downloaded on demand (Green outline)
}
