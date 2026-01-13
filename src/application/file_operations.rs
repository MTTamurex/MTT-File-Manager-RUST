use clipboard_win::{formats, Clipboard, Setter};
use std::path::{Path, PathBuf};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use crate::infrastructure::windows::recycle_bin;
use crate::infrastructure::windows::shell_operations;

/// Error type for file operations
type OpResult<T> = Result<T, String>;

/// Deletes a file or directory using Windows Shell (Recycle Bin).
pub fn delete_with_shell(path: &Path, hwnd: Option<HWND>) -> OpResult<bool> {
    let hwnd = hwnd.unwrap_or(HWND(std::ptr::null_mut()));
    let success = shell_operations::delete_item_with_shell(path, hwnd);
    if success {
        Ok(true)
    } else {
        Err("Operação cancelada ou falhou".to_string())
    }
}

/// Renames a file using Windows Shell.
pub fn rename_with_shell(path: &Path, new_name: &str, hwnd: Option<HWND>) -> OpResult<bool> {
    let hwnd = hwnd.unwrap_or(HWND(std::ptr::null_mut()));
    let success = shell_operations::rename_item_with_shell(path, new_name, hwnd);
    if success {
        Ok(true)
    } else {
        Err("Operação cancelada ou falhou".to_string())
    }
}

/// Creates a new folder with a unique name "Nova Pasta (N)".
pub fn create_new_folder(base_path: &Path) -> OpResult<PathBuf> {
    let mut new_folder_name = "Nova Pasta".to_string();
    let mut counter = 1;

    while base_path.join(&new_folder_name).exists() {
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
    if let Ok(_clip) = Clipboard::new_attempts(10) {
        if formats::Unicode
            .write_clipboard(&path.to_string_lossy())
            .is_ok()
        {
            return Ok(());
        }
    }
    Err("Falha ao abrir clipboard".to_string())
}

/// Restores file from Recycle Bin.
pub fn restore_from_recycle_bin(physical_path: &Path, original_path: &Path) -> OpResult<()> {
    recycle_bin::restore_from_recycle_bin(physical_path, original_path).map_err(|e| e.to_string())
}

/// Deletes file permanently (no Recycle Bin).
pub fn delete_permanently(physical_path: &Path) -> OpResult<()> {
    recycle_bin::delete_permanently(physical_path).map_err(|e| e.to_string())
}

/// Empties Recycle Bin.
pub fn empty_recycle_bin() -> OpResult<()> {
    recycle_bin::empty_recycle_bin().map_err(|e| e.to_string())
}

/// Creates a Windows shortcut (.lnk).
pub fn create_shortcut(target: &Path, current_path: &str) -> OpResult<PathBuf> {
    let dest_dir = target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(current_path));

    let base_name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| target.to_string_lossy().to_string());

    let mut candidate = dest_dir.join(format!("{} - Atalho.lnk", base_name));
    let mut counter = 2;
    while candidate.exists() {
        candidate = dest_dir.join(format!("{} - Atalho ({}).lnk", base_name, counter));
        counter += 1;
    }

    unsafe {
        // SAFETY: COM initialization
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| format!("CoInitializeEx failed: {e}"))?;

        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("CoCreateInstance ShellLink failed: {e}"))?;

        let wide_target: Vec<u16> = target
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

        CoUninitialize();
    }

    Ok(candidate)
}
