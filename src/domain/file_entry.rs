use crate::infrastructure::windows::system_info::DriveType;
use rust_i18n::t;
use std::path::{Path, PathBuf};

/// Volume/drive information for the "This PC" view
#[derive(Clone, Debug)]
pub struct DriveInfo {
    pub file_system: String,
    pub total_space: u64,
    pub free_space: u64,
    pub drive_type: DriveType, // Drive type (local, network, removable, etc.)
}

/// File/folder entry with cached metadata for sorting
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,                           // Cached name for fast sorting
    pub is_dir: bool,                           // Folders first
    pub size: u64,                              // Size in bytes (0 for directories)
    pub modified: u64,                          // Timestamp (seconds since UNIX_EPOCH)
    pub folder_cover: Option<PathBuf>, // First image found in the folder (for preview)
    pub drive_info: Option<DriveInfo>, // Drive metadata (optional)
    pub sync_status: SyncStatus,       // OneDrive sync status
    pub is_hidden: bool,               // Windows FILE_ATTRIBUTE_HIDDEN
    pub deletion_date: Option<String>, // Deletion date (Recycle Bin only)
    pub recycle_original_path: Option<PathBuf>, // Original path for restoration (Recycle Bin only)
}

impl FileEntry {
    pub fn from_path(path: PathBuf, is_dir: bool) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Try to read metadata, use defaults on error (locked files, etc.)
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

        // OPTIMIZATION: Lazy loading - always None initially.
        // The scan will be triggered by request_folder_scan() when the folder becomes visible.
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
            is_hidden: false,
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

/// Supported archive file extensions for navigation via Windows Shell Namespace.
/// Compound extensions (.tar.gz) must come before simple ones (.gz).
pub const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tgz", ".tar.bz2", ".tbz2", ".tar.zst", ".tzst", ".tar.xz", ".txz", ".tar", ".zip",
    ".7z", ".rar", ".gz", ".gzip",
];

/// Checks if a filename ends with an archive file extension (case-insensitive).
#[inline]
pub fn is_archive_extension(name: &str) -> bool {
    ARCHIVE_EXTENSIONS
        .iter()
        .any(|ext| ends_with_ignore_case(name, ext))
}

/// Checks if a path (already in lowercase) passes through an archive file.
/// E.g.: "C:\archive.7z\subdir\file.txt" → true
pub fn path_contains_archive_segment(path_lower: &str) -> bool {
    // PERFORMANCE: Check if path contains an archive extension followed by a separator.
    // Zero-allocation version — avoids 28 format!() calls per icon lookup.
    for ext in ARCHIVE_EXTENSIONS {
        // Look for "{ext}\" or "{ext}/" in the path
        let mut start = 0;
        while let Some(pos) = path_lower[start..].find(ext) {
            let abs_pos = start + pos + ext.len();
            if abs_pos < path_lower.len() {
                let next_byte = path_lower.as_bytes()[abs_pos];
                if next_byte == b'\\' || next_byte == b'/' {
                    return true;
                }
            }
            start = start + pos + 1;
        }
    }
    false
}

/// Returns the type label for displaying an archive file.
/// E.g.: "Arquivo ZIP", "Arquivo RAR". Returns None if not an archive file.
pub fn archive_type_label(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(t!("file_types.archive_tar_gz").to_string())
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
        Some(t!("file_types.archive_tar_bz2").to_string())
    } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") {
        Some(t!("file_types.archive_tar_zst").to_string())
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Some(t!("file_types.archive_tar_xz").to_string())
    } else if lower.ends_with(".tar") {
        Some(t!("file_types.archive_tar").to_string())
    } else if lower.ends_with(".zip") {
        Some(t!("file_types.archive_zip").to_string())
    } else if lower.ends_with(".7z") {
        Some(t!("file_types.archive_7z").to_string())
    } else if lower.ends_with(".rar") {
        Some(t!("file_types.archive_rar").to_string())
    } else if lower.ends_with(".gz") || lower.ends_with(".gzip") {
        Some(t!("file_types.archive_gz").to_string())
    } else {
        None
    }
}

/// Helper to display file type in the List view
pub fn get_file_type_string(entry: &FileEntry) -> String {
    if let Some(label) = archive_type_label(&entry.name) {
        return label;
    }
    if entry.is_dir {
        return t!("file_types.folder").to_string();
    }
    if let Some(ext) = entry.path.extension() {
        return t!("file_info.file_generic", ext = ext.to_string_lossy().to_uppercase()).to_string();
    }
    t!("file_info.file_unknown").to_string()
}

/// Sort mode
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SortMode {
    Name,
    Date,
    Size,
    Type,
    /// Total drive space (Computer View only)
    DriveTotalSpace,
    /// Free drive space (Computer View only)
    DriveFreeSpace,
}

/// View mode
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum ViewMode {
    Grid,
    List,
}

/// Icon size
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum IconSize {
    Small, // 16x16 or 32x32 (depends on DPI)
    Large, // 32x32 or 48x48
    Jumbo, // 256x256 (via Shell Image Factory)
}

/// Folder position in the listing
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum FoldersPosition {
    First, // Folders before files (default)
    Last,  // Files before folders
    Mixed, // Mixed by sort criteria
}

/// OneDrive sync status
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SyncStatus {
    #[default]
    None, // Not a cloud file / Normal
    CloudOnly,        // "Available online" (needs download)
    Syncing,          // Currently syncing (blue arrows)
    Pinned,           // "Always keep on this device" (Green check)
    LocallyAvailable, // Downloaded on demand (Green outline)
}
