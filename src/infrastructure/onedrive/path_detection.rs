use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

/// Cached set of known Windows special folder paths (lowercased, no trailing `\`).
/// Populated once at startup via `SHGetKnownFolderPath`.
static SPECIAL_FOLDER_PATHS: OnceLock<HashSet<String>> = OnceLock::new();

/// Maps lowercased special folder paths → i18n key suffixes (e.g. "desktop", "documents").
/// Used by `special_folder_display_name()` to provide translated display names.
static SPECIAL_FOLDER_KEYS: OnceLock<HashMap<String, &'static str>> = OnceLock::new();

pub(super) fn init_onedrive_paths() {
    super::ONEDRIVE_ROOTS.get_or_init(|| {
        let mut roots = Vec::new();
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Ok(path) = std::env::var(var) {
                if !path.is_empty() {
                    roots.push(path.to_lowercase());
                }
            }
        }
        log::info!("[OneDrive] Detected roots: {:?}", roots);
        roots
    });

    // Resolve actual special folder paths via Windows Shell API (locale-independent).
    SPECIAL_FOLDER_PATHS.get_or_init(|| {
        let mut set = HashSet::new();
        let mut keys = HashMap::new();
        resolve_known_folder_paths(&mut set, &mut keys);
        // Also include OneDrive root folders as special (they have their own shell icon).
        if let Some(roots) = super::ONEDRIVE_ROOTS.get() {
            for root in roots {
                let normalized = root.trim_end_matches('\\').to_string();
                set.insert(normalized.clone());
                keys.insert(normalized, "onedrive");
            }
        }
        SPECIAL_FOLDER_KEYS.get_or_init(|| {
            log::info!("[OneDrive] SPECIAL_FOLDER_KEYS: {:?}", keys);
            keys
        });
        log::info!("[OneDrive] Special folder paths: {:?}", set);
        set
    });
}

/// Uses `SHGetKnownFolderPath` to collect the real filesystem paths of well-known
/// Windows special folders. This handles any locale (Portuguese, English, etc.)
/// and also covers OneDrive "Known Folder Move" redirections.
#[cfg(target_os = "windows")]
fn resolve_known_folder_paths(out: &mut HashSet<String>, keys: &mut HashMap<String, &'static str>) {
    use windows::Win32::UI::Shell::KF_FLAG_DONT_VERIFY;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_SavedGames, FOLDERID_Videos, SHGetKnownFolderPath,
    };

    let folder_ids: [(&windows::core::GUID, &'static str); 7] = [
        (&FOLDERID_Desktop, "desktop"),
        (&FOLDERID_Documents, "documents"),
        (&FOLDERID_Downloads, "downloads"),
        (&FOLDERID_Music, "music"),
        (&FOLDERID_Pictures, "pictures"),
        (&FOLDERID_Videos, "videos"),
        (&FOLDERID_SavedGames, "saved_games"),
    ];

    for (id, key) in folder_ids {
        unsafe {
            if let Ok(pwstr) = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None) {
                let path = pwstr.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.0 as *const _));
                if !path.is_empty() {
                    let normalized = path.to_lowercase().trim_end_matches('\\').to_string();
                    out.insert(normalized.clone());
                    keys.insert(normalized, key);
                }
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn resolve_known_folder_paths(
    _out: &mut HashSet<String>,
    _keys: &mut HashMap<String, &'static str>,
) {
}

pub(super) fn is_onedrive_path(path: &Path) -> bool {
    let path_lower = path.to_string_lossy().to_lowercase();
    super::ONEDRIVE_ROOTS
        .get()
        .map(|roots| roots.iter().any(|r| path_lower.starts_with(r)))
        .unwrap_or(false)
}

/// Returns true if `path` matches a known Windows special folder
/// (Documents, Pictures, Desktop, Downloads, Music, Videos, etc.).
///
/// Uses paths resolved via `SHGetKnownFolderPath` at startup — handles any
/// locale and OneDrive "Known Folder Move" redirections automatically.
///
/// Pure HashSet lookup — no I/O. Safe to call per item in the render loop.
/// Returns the set of resolved special folder paths (lowercased, no trailing `\`).
/// Used to pre-extract their icons at startup.
pub(super) fn special_folder_paths() -> Vec<String> {
    SPECIAL_FOLDER_PATHS
        .get()
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn is_special_icon_folder(path: &Path) -> bool {
    let path_lower = path.to_string_lossy().to_lowercase();
    let path_norm = path_lower.trim_end_matches('\\');

    SPECIAL_FOLDER_PATHS
        .get()
        .map(|set| set.contains(path_norm))
        .unwrap_or(false)
}

/// Returns a translated display name for a known special folder path,
/// or `None` if the path is not a recognized special folder.
pub(super) fn special_folder_display_name(path: &Path) -> Option<String> {
    let path_lower = path.to_string_lossy().to_lowercase();
    let path_norm = path_lower.trim_end_matches('\\');

    SPECIAL_FOLDER_KEYS
        .get()
        .and_then(|map| map.get(path_norm))
        .map(|key| translate_special_folder_key(key))
}

fn translate_special_folder_key(key: &str) -> String {
    use rust_i18n::t;
    match key {
        "desktop" => t!("special_folders.desktop"),
        "documents" => t!("special_folders.documents"),
        "downloads" => t!("special_folders.downloads"),
        "music" => t!("special_folders.music"),
        "pictures" => t!("special_folders.pictures"),
        "videos" => t!("special_folders.videos"),
        "saved_games" => t!("special_folders.saved_games"),
        "onedrive" => t!("special_folders.onedrive"),
        _ => return key.to_string(),
    }
    .to_string()
}
