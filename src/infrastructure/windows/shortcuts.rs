//! Windows `.lnk` shortcut resolution helpers.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{Interface, PCWSTR};
use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::System::Com::{
    CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLGP_UNCPRIORITY};

/// Resolves a Windows shortcut to its raw target path.
pub fn resolve_shortcut_target(shortcut_path: &Path) -> Option<PathBuf> {
    let _com =
        crate::infrastructure::windows::recycle_bin::ComApartmentGuard::init_sta_best_effort();

    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let persist: IPersistFile = link.cast().ok()?;
        let shortcut_wide: Vec<u16> = shortcut_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        persist
            .Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READ)
            .ok()?;

        let mut target = vec![0u16; 32_768];
        link.GetPath(&mut target, std::ptr::null_mut(), SLGP_UNCPRIORITY.0 as u32)
            .ok()?;
        let nul = target
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(target.len());
        if nul == 0 {
            return None;
        }

        let target_text = String::from_utf16_lossy(&target[..nul]);
        Some(PathBuf::from(
            super::environment::expand_environment_strings(&target_text),
        ))
    }
}

/// Returns true if the path exists and is a directory.
///
/// `GetFileAttributesW` reads cached metadata only — no file recall, no disk I/O.
pub fn fast_is_directory(path: &Path) -> bool {
    let path_wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let attrs = unsafe { GetFileAttributesW(PCWSTR(path_wide.as_ptr())) };
    attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY.0) != 0
}

/// Returns the target of a `.lnk` shortcut when the target is a directory.
///
/// Because a folder shortcut cannot execute code, callers navigate in-app to the
/// target instead of shelling out, keeping the opening behavior independent of
/// the system default file manager registration.
pub fn resolve_folder_shortcut_target(shortcut_path: &Path) -> Option<PathBuf> {
    let is_lnk = shortcut_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
    if !is_lnk {
        return None;
    }

    let target = resolve_shortcut_target(shortcut_path)?;
    if !fast_is_directory(&target) {
        return None;
    }

    log::debug!(
        "[Shortcuts] Folder shortcut: {} -> {}",
        shortcut_path.display(),
        target.display()
    );
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_shortcut(link_path: &Path, target: &Path) {
        let _com =
            crate::infrastructure::windows::recycle_bin::ComApartmentGuard::init_sta_best_effort();

        fn wide(path: &Path) -> Vec<u16> {
            path.as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect()
        }

        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).expect("create IShellLinkW");
            link.SetPath(PCWSTR(wide(target).as_ptr())).expect("SetPath");
            let persist: IPersistFile = link.cast().expect("cast IPersistFile");
            persist
                .Save(PCWSTR(wide(link_path).as_ptr()), false)
                .expect("Save shortcut");
        }
    }

    #[test]
    fn folder_shortcut_resolves_to_target_directory() {
        let root = tempfile::tempdir().expect("tempdir");
        let target_dir = root.path().join("mirror");
        std::fs::create_dir_all(&target_dir).expect("create target dir");
        let link_path = root.path().join("folder.lnk");
        write_shortcut(&link_path, &target_dir);

        let resolved = resolve_folder_shortcut_target(&link_path).expect("resolve target");
        assert_eq!(resolved, target_dir);
    }

    #[test]
    fn shortcut_to_file_returns_none() {
        let root = tempfile::tempdir().expect("tempdir");
        let target_file = root.path().join("document.txt");
        std::fs::write(&target_file, "data").expect("write file");
        let link_path = root.path().join("file.lnk");
        write_shortcut(&link_path, &target_file);

        assert!(resolve_folder_shortcut_target(&link_path).is_none());
    }

    #[test]
    fn non_shortcut_paths_return_none() {
        let root = tempfile::tempdir().expect("tempdir");

        let plain_file = root.path().join("file.txt");
        std::fs::write(&plain_file, "data").expect("write file");
        assert!(resolve_folder_shortcut_target(&plain_file).is_none());

        let dir = root.path().join("real_dir");
        std::fs::create_dir_all(&dir).expect("create dir");
        assert!(resolve_folder_shortcut_target(&dir).is_none());
    }
}
