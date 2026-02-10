use clipboard_win::{formats, Clipboard, Setter};
use std::path::{Path, PathBuf};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use crate::infrastructure::security::{sanitize_path, SecurityConfig};
use crate::infrastructure::windows as windows_infra;
use crate::infrastructure::windows::recycle_bin;
use crate::infrastructure::windows::shell_operations;

/// Error type for file operations.
type OpResult<T> = Result<T, String>;

/// RAII guard for COM apartment initialization.
struct ComApartmentGuard;

impl ComApartmentGuard {
    fn init_sta() -> OpResult<Self> {
        unsafe {
            // SAFETY: Called on the current thread before COM operations.
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| format!("CoInitializeEx failed: {e}"))?;
        }
        Ok(Self)
    }
}

impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        unsafe {
            // SAFETY: Balanced with successful CoInitializeEx in init_sta.
            CoUninitialize();
        }
    }
}

fn operation_security_config() -> SecurityConfig {
    let allowed_drives = ('A'..='Z').map(|c| format!("{}:", c)).collect();
    SecurityConfig {
        allowed_drives,
        // Windows paths commonly include junctions/reparse points in valid locations.
        allow_symlinks: true,
        ..SecurityConfig::default()
    }
}

fn should_bypass_sanitization(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.starts_with("shell:")
        || s.starts_with("\\\\")
        || windows_infra::is_shell_navigation_path(path, false)
}

fn sanitize_operation_path(path: &Path) -> OpResult<PathBuf> {
    if should_bypass_sanitization(path) {
        return Ok(path.to_path_buf());
    }
    sanitize_path(path, &operation_security_config()).map_err(|e| e.to_string())
}

/// Deletes a file or directory using Windows Shell (Recycle Bin).
///
/// # Warning
/// This is a BLOCKING operation. Do not use on the UI thread.
/// Use `ImageViewerApp::delete_with_shell_for_paths` instead which uses a worker.
#[deprecated(note = "Blocking operation. Use ImageViewerApp::file_op_sender instead.")]
pub fn delete_with_shell(path: &Path, hwnd: Option<HWND>) -> OpResult<bool> {
    let valid_path = sanitize_operation_path(path)?;
    let hwnd = hwnd.unwrap_or(HWND(std::ptr::null_mut()));
    let success = shell_operations::delete_item_with_shell(&valid_path, hwnd);
    if success {
        Ok(true)
    } else {
        Err("Operacao cancelada ou falhou".to_string())
    }
}

/// Opens a file with its default application.
pub fn open_with_shell(path: &Path, _hwnd: Option<HWND>) -> OpResult<()> {
    let valid_path = sanitize_operation_path(path)?;
    shell_operations::open_with_shell(&valid_path);
    Ok(())
}

/// Renames a file using Windows Shell.
///
/// # Warning
/// This is a BLOCKING operation. Do not use on the UI thread.
/// Use `ImageViewerApp::rename_with_shell` instead which uses a worker.
#[deprecated(note = "Blocking operation. Use ImageViewerApp::file_op_sender instead.")]
pub fn rename_with_shell(path: &Path, new_name: &str, hwnd: Option<HWND>) -> OpResult<bool> {
    if new_name.contains('\0')
        || new_name.contains('\\')
        || new_name.contains('/')
        || new_name == "."
        || new_name == ".."
    {
        return Err("Nome invalido para renomear".to_string());
    }

    let valid_path = sanitize_operation_path(path)?;
    let hwnd = hwnd.unwrap_or(HWND(std::ptr::null_mut()));
    let success = shell_operations::rename_item_with_shell(&valid_path, new_name, hwnd);
    if success {
        Ok(true)
    } else {
        Err("Operacao cancelada ou falhou".to_string())
    }
}

/// Creates a new folder with a unique name "Nova Pasta (N)".
pub fn create_new_folder(base_path: &Path) -> OpResult<PathBuf> {
    let base_path = sanitize_operation_path(base_path)?;
    let mut new_folder_name = "Nova Pasta".to_string();
    let mut counter = 1;

    // Use fast_path_exists() to avoid blocking OneDrive recalls.
    while crate::infrastructure::onedrive::fast_path_exists(&base_path.join(&new_folder_name)) {
        counter += 1;
        new_folder_name = format!("Nova Pasta ({})", counter);
    }

    let full_path = base_path.join(&new_folder_name);

    match std::fs::create_dir(&full_path) {
        Ok(_) => Ok(full_path),
        Err(e) => Err(format!("Erro ao criar pasta: {}", e)),
    }
}

/// Copy path to clipboard as text.
pub fn copy_path_to_clipboard(path: &Path) -> OpResult<()> {
    let valid_path = sanitize_operation_path(path)?;
    if let Ok(_clip) = Clipboard::new_attempts(10) {
        if formats::Unicode
            .write_clipboard(&valid_path.to_string_lossy())
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("Falha ao abrir clipboard".to_string())
}

/// Restores file from Recycle Bin.
pub fn restore_from_recycle_bin(physical_path: &Path, original_path: &Path) -> OpResult<()> {
    let valid_physical = sanitize_operation_path(physical_path)?;
    let valid_original = sanitize_operation_path(original_path)?;
    recycle_bin::restore_from_recycle_bin(&valid_physical, &valid_original)
        .map_err(|e| e.to_string())
}

/// Deletes file permanently (no Recycle Bin).
pub fn delete_permanently(
    physical_path: &Path,
    hwnd: windows::Win32::Foundation::HWND,
) -> OpResult<()> {
    let valid_physical = sanitize_operation_path(physical_path)?;
    recycle_bin::delete_permanently(&valid_physical, hwnd).map_err(|e| e.to_string())
}

/// Empties Recycle Bin.
pub fn empty_recycle_bin(hwnd: windows::Win32::Foundation::HWND) -> OpResult<()> {
    recycle_bin::empty_recycle_bin(hwnd).map_err(|e| e.to_string())
}

/// Creates a Windows shortcut (.lnk).
pub fn create_shortcut(target: &Path, current_path: &str) -> OpResult<PathBuf> {
    let valid_target = sanitize_operation_path(target)?;

    let fallback_dir = PathBuf::from(current_path);
    let raw_dest_dir = valid_target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(fallback_dir);
    let dest_dir = sanitize_operation_path(&raw_dest_dir)?;

    let base_name = valid_target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| valid_target.to_string_lossy().to_string());

    let mut candidate = dest_dir.join(format!("{} - Atalho.lnk", base_name));
    let mut counter = 2;
    while crate::infrastructure::onedrive::fast_path_exists(&candidate) {
        candidate = dest_dir.join(format!("{} - Atalho ({}).lnk", base_name, counter));
        counter += 1;
    }

    let _com = ComApartmentGuard::init_sta()?;

    unsafe {
        // SAFETY: COM apartment is initialized for this thread by _com guard.
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("CoCreateInstance ShellLink failed: {e}"))?;

        let wide_target: Vec<u16> = valid_target
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let wide_workdir: Vec<u16> = dest_dir
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        link.SetPath(PCWSTR(wide_target.as_ptr()))
            .map_err(|e| format!("SetPath failed: {e}"))?;
        link.SetWorkingDirectory(PCWSTR(wide_workdir.as_ptr()))
            .map_err(|e| format!("SetWorkingDirectory failed: {e}"))?;

        let persist: IPersistFile = link
            .cast()
            .map_err(|e| format!("IPersistFile cast failed: {e}"))?;

        let wide_dest: Vec<u16> = candidate
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        persist
            .Save(PCWSTR(wide_dest.as_ptr()), true)
            .map_err(|e| format!("Persist Save failed: {e}"))?;
    }

    Ok(candidate)
}
